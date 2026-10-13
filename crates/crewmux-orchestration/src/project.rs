// Derived from openrig packages/daemon/src/domain/workflow-projector.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Close the frontier and project the successor on one draft.
//!
//! The inputs are not mutated. A [`Phase`] crash returns [`Error::Crashed`]
//! and the caller keeps the published pair.

use crewmux_core::Timestamp;
use crewmux_messaging::{
    Actor, NewItem, QueueBook, QueueRepo, QueueState, SessionRef, Update,
};
use crewmux_spec::workflow::{StepSpec, WorkflowSpec};
use crewmux_types::QItemId;
use serde_json::Value;

use crate::deadline::exceeds_max_hops;
use crate::error::Error;
use crate::exit::Exit;
use crate::keepalive::{self, disarm};
use crate::model::{Decision, Engine, Instance, Phase, Status, Trail};
use crate::packet;
use crate::route::{reconcile_explicit, resolve_next_step, resolve_owner};
use crate::world::World;

/// One `project` call.
pub struct ProjectRequest {
    /// Instance.
    pub instance_id: String,
    /// Frontier packet.
    pub packet_id: String,
    /// Exit.
    pub exit: Exit,
    /// Actor session.
    pub actor: String,
    /// Result note.
    pub result_note: Option<String>,
    /// Waiting blocker. Default `external-gate`.
    pub blocked_on: Option<String>,
    /// Closure evidence.
    pub evidence: Option<Value>,
    /// Explicit next owner.
    pub next_owner: Option<String>,
    /// Stale writers pass the version they read.
    pub expected_version: Option<u32>,
}

/// What `project` did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Projection {
    /// Closed packet.
    pub closure_prior: String,
    /// Workflow exit that committed.
    pub closure_reason: String,
    /// Successor, if any.
    pub next_qitem: Option<String>,
    /// Successor owner.
    pub next_owner: Option<String>,
    /// Successor step.
    pub next_step: Option<String>,
    /// Event types, in order.
    pub events: Vec<&'static str>,
    /// True when the waiting replay wrote nothing.
    pub absorbed: bool,
}

/// Context for one projection.
pub struct ProjectCtx<'a> {
    /// Inventory.
    pub world: &'a World,
    /// Now.
    pub now: Timestamp,
    /// Crash checkpoint.
    pub crash: Option<Phase>,
    /// Called between phases. Sees the published pair, which this function does not write.
    pub observe: &'a mut dyn FnMut(),
}

/// Project `request` onto clones of `engine` and `book`.
///
/// # Errors
///
/// Returns a typed rejection before mutation, or [`Error::Crashed`].
pub fn project_step(
    engine: &Engine,
    book: &QueueBook,
    request: &ProjectRequest,
    ctx: &mut ProjectCtx<'_>,
) -> Result<(Engine, QueueBook, Projection), Error> {
    let spec = load(engine, book, request)?;
    let instance = engine
        .instance(&request.instance_id)
        .ok_or_else(|| missing(&request.instance_id))?;
    let step = current_step(&spec.0, instance)?;
    if let Some(absorbed) = absorb(engine, request, &step.id) {
        return Ok((engine.clone(), book.clone(), absorbed));
    }
    let plan = plan_route(&spec.0, instance, step, request, ctx.world)?;
    let mut draft_engine = engine.clone();
    let mut draft_book = book.clone();
    let next_id = apply(
        &mut draft_engine,
        &mut draft_book,
        request,
        &spec.0,
        step,
        &plan,
        ctx,
    )?;
    let projection = Projection {
        closure_prior: request.packet_id.clone(),
        closure_reason: plan.exit.as_str().to_owned(),
        next_qitem: next_id,
        next_owner: plan.owner.clone(),
        next_step: plan.next_step.clone(),
        events: plan.events.clone(),
        absorbed: false,
    };
    Ok((draft_engine, draft_book, projection))
}

struct Loaded(WorkflowSpec);

fn load(engine: &Engine, book: &QueueBook, request: &ProjectRequest) -> Result<Loaded, Error> {
    let instance = engine
        .instance(&request.instance_id)
        .ok_or_else(|| missing(&request.instance_id))?;
    if !instance.status.is_live() {
        return Err(Error::reject(
            "instance_not_active",
            format!(
                "workflow instance {} has status {}; only active|waiting instances accept project",
                instance.id,
                instance.status.as_str()
            ),
            409,
        ));
    }
    if request.expected_version.is_some_and(|expected| expected != instance.version) {
        return Err(Error::reject(
            "instance_version_conflict",
            format!(
                "workflow instance {} version conflict: expected {}, actual {}",
                instance.id,
                request.expected_version.unwrap_or(instance.version),
                instance.version
            ),
            409,
        ));
    }
    if !instance.frontier.iter().any(|id| id.as_str() == request.packet_id) {
        let frontier = serde_json::to_string(
            &instance
                .frontier
                .iter()
                .map(crewmux_types::QItemId::as_str)
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".to_owned());
        return Err(Error::reject(
            "packet_not_on_frontier",
            format!(
                "qitem {} is not in workflow instance {} frontier {frontier}; either the packet is already closed or belongs to a different instance",
                request.packet_id, instance.id
            ),
            409,
        ));
    }
    let id = QItemId::new(&request.packet_id).map_err(|_| {
        Error::reject("packet_not_found", format!("qitem {} not found in queue_items", request.packet_id), 404)
    })?;
    if book.get(&id).is_none() {
        return Err(Error::reject(
            "packet_not_found",
            format!("qitem {} not found in queue_items", request.packet_id),
            404,
        ));
    }
    let key = Engine::spec_key(&instance.workflow_name, &instance.workflow_version);
    let cached = engine.specs.get(&key).ok_or_else(|| {
        Error::reject(
            "spec_not_cached",
            format!(
                "workflow spec {}@{} is not in the spec cache. Re-run validate to refresh the cache.",
                instance.workflow_name, instance.workflow_version
            ),
            409,
        )
    })?;
    let spec = crewmux_spec::parse_workflow(&cached.yaml, &cached.source_path).map_err(|err| {
        Error::reject(err.code, err.message, 400)
    })?;
    if spec.steps.iter().any(|step| step.depends_on.is_some()) {
        return Err(Error::reject(
            "frontier_binding_indeterminate",
            format!(
                "workflow instance {} has a dependency graph but frontier packet {} has no unique step binding; refusing to infer from current_step_id",
                instance.id, request.packet_id
            ),
            409,
        ));
    }
    Ok(Loaded(spec))
}

fn missing(id: &str) -> Error {
    Error::reject("instance_not_found", format!("workflow instance {id} not found"), 404)
}

fn current_step<'a>(spec: &'a WorkflowSpec, instance: &Instance) -> Result<&'a StepSpec, Error> {
    if let Some(id) = instance.current_step_id.as_deref()
        && let Some(step) = spec.steps.iter().find(|step| step.id == id)
    {
        return Ok(step);
    }
    if instance.current_step_id.is_none()
        && let Some(step) = spec.steps.first()
    {
        return Ok(step);
    }
    Err(Error::reject(
        "current_step_unknown",
        format!(
            "workflow instance {} has no resolvable current step (current_step_id={}). The instance may be in a terminal state, or the spec {}@{} may have changed shape mid-instance.",
            instance.id,
            serde_json::to_string(&instance.current_step_id).unwrap_or_else(|_| "null".to_owned()),
            spec.id,
            spec.version_label
        ),
        409,
    ))
}

fn absorb(engine: &Engine, request: &ProjectRequest, step_id: &str) -> Option<Projection> {
    if request.exit != Exit::Waiting {
        return None;
    }
    let instance = engine.instance(&request.instance_id)?;
    if instance.status != Status::Waiting {
        return None;
    }
    let stored = instance.last_decision.as_ref()?;
    if stored.exit != "waiting"
        || stored.closed_packet != request.packet_id
        || stored.current_step != step_id
        || stored.actor_session != request.actor
        || stored.result_note != request.result_note
    {
        return None;
    }
    let stored_blocker = stored.blocked_on.clone().unwrap_or_else(|| "external-gate".to_owned());
    let input_blocker = request.blocked_on.clone().unwrap_or_else(|| "external-gate".to_owned());
    if stored_blocker != input_blocker {
        return None;
    }
    let row = engine.trails.iter().find(|row| {
        row.instance_id == request.instance_id
            && row.prior_qitem_id == request.packet_id
            && row.closure_reason == "waiting"
    })?;
    if row.closure_evidence != request.evidence {
        return None;
    }
    Some(Projection {
        closure_prior: request.packet_id.clone(),
        closure_reason: "waiting".to_owned(),
        next_qitem: None,
        next_owner: None,
        next_step: None,
        events: Vec::new(),
        absorbed: true,
    })
}

struct Plan {
    exit: Exit,
    note: Option<String>,
    routes: bool,
    owner: Option<String>,
    next_step: Option<String>,
    branch: Option<String>,
    events: Vec<&'static str>,
}

#[expect(
    clippy::too_many_lines,
    reason = "exit checks, the hop guard, and owner resolution stay in one planning pass"
)]
fn plan_route(
    spec: &WorkflowSpec,
    instance: &Instance,
    step: &StepSpec,
    request: &ProjectRequest,
    world: &World,
) -> Result<Plan, Error> {
    if step.allowed_exits.as_ref().is_some_and(|exits| {
        !exits.is_empty() && !exits.iter().any(|exit| exit == request.exit.as_str())
    }) {
        let rendered = step
            .allowed_exits
            .as_ref()
            .and_then(|exits| serde_json::to_string(exits).ok())
            .unwrap_or_else(|| "[]".to_owned());
        return Err(Error::reject(
            "exit_not_allowed",
            format!(
                "step \"{}\" allows exits {rendered}; got \"{}\". Either close with a permitted exit, or amend the spec.",
                step.id,
                request.exit.as_str()
            ),
            400,
        ));
    }
    let mapped = step.next_hop.as_ref().and_then(|hop| hop.mapped(request.exit.as_str()));
    let branch_routed = mapped.is_some();
    let next = resolve_next_step(spec, step, Some(request.exit.as_str()));
    if branch_routed && next.is_none() {
        return Err(Error::reject(
            "branch_target_missing",
            format!(
                "step \"{}\" maps exit \"{}\" to step \"{}\" but the spec {}@{} has no such step. Re-validate the spec file.",
                step.id,
                request.exit.as_str(),
                mapped.unwrap_or(""),
                spec.id,
                spec.version_label
            ),
            409,
        ));
    }
    let will_route = branch_routed || request.exit == Exit::Handoff;
    let tripped = will_route
        && exceeds_max_hops(
            instance.hop_count,
            instance.hops_baseline,
            spec.loop_guards.as_ref().and_then(|guards| guards.max_hops),
        );
    let exit = if tripped { Exit::Failed } else { request.exit };
    let note = if tripped {
        Some(format!(
            "max_hops_exceeded: hop {} would exceed loop_guards.max_hops={}",
            instance.hop_count.saturating_add(1),
            spec.loop_guards.as_ref().and_then(|guards| guards.max_hops).unwrap_or(0)
        ))
    } else {
        request.result_note.clone()
    };
    let routes = will_route && !tripped;
    let (owner, next_step, branch) = if routes {
        let next = next.ok_or_else(|| {
            Error::reject(
                "no_next_step",
                format!(
                    "workflow instance {} reached terminal step \"{}\" but exit was handoff; use exit=done for terminal steps or extend the spec with a next step",
                    instance.id, step.id
                ),
                400,
            )
        })?;
        let owner = owner_for(spec, next, request, world, instance.bound_rig.is_some())?;
        let branch = if branch_routed { Some(next.id.clone()) } else { None };
        (Some(owner), Some(next.id.clone()), branch)
    } else {
        (None, None, None)
    };
    let mut events = vec!["workflow.step_closed"];
    if routes {
        events.push("queue.created");
        events.push("workflow.next_qitem_projected");
    }
    if exit == Exit::Done {
        events.push("workflow.completed");
    } else if exit == Exit::Failed {
        events.push("workflow.failed");
    }
    Ok(Plan {
        exit,
        note,
        routes,
        owner,
        next_step,
        branch,
        events,
    })
}

fn owner_for(
    spec: &WorkflowSpec,
    step: &StepSpec,
    request: &ProjectRequest,
    world: &World,
    bound: bool,
) -> Result<String, Error> {
    if let Some(explicit) = request.next_owner.as_deref() {
        reconcile_explicit(step, explicit, world)?;
        return Ok(explicit.to_owned());
    }
    match resolve_owner(spec, step, world, bound)? {
        Some(owner) => Ok(owner),
        None => Err(Error::reject(
            "next_owner_unresolved",
            format!(
                "cannot resolve next owner for step \"{}\" (role \"{}\"); supply nextOwnerSession explicitly or add preferred_targets to the role in the spec",
                step.id, step.actor_role
            ),
            400,
        )),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one projection threads the draft engine, the draft book, and the plan"
)]
fn apply(
    engine: &mut Engine,
    book: &mut QueueBook,
    request: &ProjectRequest,
    spec: &WorkflowSpec,
    step: &StepSpec,
    plan: &Plan,
    ctx: &mut ProjectCtx<'_>,
) -> Result<Option<String>, Error> {
    let instance_id = request.instance_id.clone();
    close_frontier(book, request, plan, ctx.now)?;
    checkpoint(ctx, Phase::AfterClose)?;
    let next_id = create_next(book, request, spec, step, plan, ctx.now)?;
    checkpoint(ctx, Phase::AfterCreate)?;
    move_frontier(engine, &instance_id, request, step, plan, next_id.as_deref(), ctx.now)?;
    checkpoint(ctx, Phase::AfterFrontier)?;
    append_trail(engine, request, step, plan, next_id.as_deref(), ctx.now);
    checkpoint(ctx, Phase::AfterTrail)?;
    touch_keepalive(engine, &instance_id, plan);
    checkpoint(ctx, Phase::AfterKeepalive)?;
    if let Some(id) = next_id.as_deref() {
        let qid = QItemId::new(id)?;
        book.append_note(
            &qid,
            "workflow.nudge",
            Some("workflow nudge"),
            Some("recorded"),
            ctx.now,
        )?;
    }
    Ok(next_id)
}

fn checkpoint(ctx: &mut ProjectCtx<'_>, phase: Phase) -> Result<(), Error> {
    (ctx.observe)();
    if ctx.crash == Some(phase) {
        return Err(Error::Crashed(phase.as_str()));
    }
    Ok(())
}

fn close_frontier(
    book: &mut QueueBook,
    request: &ProjectRequest,
    plan: &Plan,
    now: Timestamp,
) -> Result<(), Error> {
    let id = QItemId::new(&request.packet_id)?;
    let actor = session(&request.actor)?;
    let close = closure(plan, request)?;
    book.update_within_transaction(
        &id,
        Update {
            state: Some(close.state),
            actor: Actor::Session(actor),
            note: close.note,
            closure_reason: Some(close.reason),
            closure_target: close.target,
            reopen: false,
            blocked_on: close.blocked,
        },
        now,
    )?;
    Ok(())
}

struct Close {
    state: QueueState,
    reason: String,
    target: Option<String>,
    blocked: Option<SessionRef>,
    note: String,
}

fn closure(plan: &Plan, request: &ProjectRequest) -> Result<Close, Error> {
    match plan.exit {
        Exit::Handoff => {
            let owner = plan.owner.clone().unwrap_or_default();
            Ok(Close {
                state: QueueState::HandedOff,
                reason: "handed_off_to".to_owned(),
                target: Some(owner.clone()),
                blocked: None,
                note: format!("workflow handoff to {owner}"),
            })
        }
        Exit::Waiting => {
            let blocker = request.blocked_on.clone().unwrap_or_else(|| "external-gate".to_owned());
            Ok(Close {
                state: QueueState::Blocked,
                reason: "blocked_on".to_owned(),
                target: Some(blocker.clone()),
                blocked: Some(session(&blocker)?),
                note: format!("workflow waiting on {blocker}"),
            })
        }
        Exit::Done => Ok(Close {
            state: QueueState::Done,
            reason: "no-follow-on".to_owned(),
            target: None,
            blocked: None,
            note: match request.result_note.as_deref() {
                Some(note) => format!("workflow done: {note}"),
                None => "workflow done".to_owned(),
            },
        }),
        Exit::Failed => {
            let target = plan
                .note
                .clone()
                .or(request.result_note.clone())
                .unwrap_or_else(|| "workflow_step_failed".to_owned());
            Ok(Close {
                state: QueueState::Done,
                reason: "denied".to_owned(),
                target: Some(target),
                blocked: None,
                note: match plan.note.as_deref() {
                    Some(note) => format!("workflow failed: {note}"),
                    None => "workflow failed".to_owned(),
                },
            })
        }
    }
}

fn create_next(
    book: &mut QueueBook,
    request: &ProjectRequest,
    spec: &WorkflowSpec,
    step: &StepSpec,
    plan: &Plan,
    now: Timestamp,
) -> Result<Option<String>, Error> {
    if !plan.routes {
        return Ok(None);
    }
    let owner = plan.owner.clone().unwrap_or_default();
    let next = spec.steps.iter().find(|candidate| plan.next_step.as_deref() == Some(candidate.id.as_str()));
    let Some(next) = next else {
        return Ok(None);
    };
    let prior = QItemId::new(&request.packet_id)?;
    let minted = QItemId::generate();
    let instance = request.instance_id.as_str();
    let body = packet::handoff_body(
        spec,
        instance,
        step,
        next,
        &request.actor,
        plan.note.as_deref(),
        minted.as_str(),
        &owner,
    );
    let created = book.create(
        NewItem {
            id: Some(minted),
            source: session(&request.actor)?,
            destination: session(&owner)?,
            body,
            priority: 0,
            tier: Some("mode2".to_owned()),
            tags: vec![
                "workflow".to_owned(),
                "handoff".to_owned(),
                format!("workflow:{}", spec.id),
                format!("instance:{instance}"),
            ],
            handed_off_from: Some(prior),
            last_nudge: None,
            explicit_human: false,
            fingerprint: String::new(),
        },
        now,
    )?;
    Ok(Some(created.id.as_str().to_owned()))
}

#[expect(
    clippy::too_many_arguments,
    reason = "frontier, status, and step are decided from the exit and the successor together"
)]
fn move_frontier(
    engine: &mut Engine,
    instance_id: &str,
    request: &ProjectRequest,
    step: &StepSpec,
    plan: &Plan,
    next_id: Option<&str>,
    now: Timestamp,
) -> Result<(), Error> {
    let instance = engine.instance_mut(instance_id).ok_or_else(|| missing(instance_id))?;
    let remaining: Vec<QItemId> = instance
        .frontier
        .iter()
        .filter(|id| id.as_str() != request.packet_id)
        .cloned()
        .collect();
    let (frontier, status, step_id, completed) = next_state(instance, request, plan, next_id, remaining)?;
    instance.frontier = frontier;
    instance.status = status;
    instance.current_step_id = step_id;
    instance.completed_at = completed.then_some(now);
    if plan.routes {
        instance.hop_count = instance.hop_count.saturating_add(1);
    }
    instance.version = instance.version.saturating_add(1);
    instance.last_decision = Some(Decision {
        exit: plan.exit.as_str().to_owned(),
        actor_session: request.actor.clone(),
        closed_packet: request.packet_id.clone(),
        next_packet: next_id.map(str::to_owned),
        result_note: plan.note.clone(),
        blocked_on: request.blocked_on.clone(),
        current_step: step.id.clone(),
        branch_taken: plan.branch.clone(),
    });
    Ok(())
}

fn next_state(
    instance: &Instance,
    request: &ProjectRequest,
    plan: &Plan,
    next_id: Option<&str>,
    remaining: Vec<QItemId>,
) -> Result<(Vec<QItemId>, Status, Option<String>, bool), Error> {
    if let Some(id) = next_id {
        let mut frontier = remaining;
        frontier.push(QItemId::new(id)?);
        return Ok((frontier, Status::Active, plan.next_step.clone(), false));
    }
    if plan.exit == Exit::Waiting {
        let mut frontier = remaining;
        frontier.push(QItemId::new(&request.packet_id)?);
        return Ok((
            frontier,
            Status::Waiting,
            instance.current_step_id.clone(),
            false,
        ));
    }
    if plan.exit == Exit::Failed {
        return Ok((remaining, Status::Failed, None, true));
    }
    Ok((remaining, Status::Completed, None, true))
}

fn append_trail(
    engine: &mut Engine,
    request: &ProjectRequest,
    step: &StepSpec,
    plan: &Plan,
    next_id: Option<&str>,
    now: Timestamp,
) {
    engine.trails.push(Trail {
        id: crewmux_types::WorkflowInstanceId::generate().to_string(),
        instance_id: request.instance_id.clone(),
        step_id: step.id.clone(),
        step_role: step.actor_role.clone(),
        closed_at: now,
        closure_reason: plan.exit.as_str().to_owned(),
        closure_evidence: request.evidence.clone(),
        actor_session: request.actor.clone(),
        next_qitem_id: next_id.map(str::to_owned),
        prior_qitem_id: request.packet_id.clone(),
    });
}

fn touch_keepalive(engine: &mut Engine, instance_id: &str, plan: &Plan) {
    if plan.routes
        && let Some(owner) = plan.owner.as_deref()
    {
        keepalive::arm(engine, instance_id, owner);
        return;
    }
    if let Some(instance) = engine.instance(instance_id)
        && matches!(instance.status, Status::Completed | Status::Failed)
    {
        let reason = if instance.status == Status::Failed {
            "workflow_failed"
        } else {
            "workflow_completed"
        };
        disarm(engine, instance_id, reason);
    }
}

fn session(text: &str) -> Result<SessionRef, Error> {
    SessionRef::new(text).map_err(Error::from)
}
