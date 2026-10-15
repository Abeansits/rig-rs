// Derived from openrig packages/daemon/src/domain/workflow-runtime.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Instantiate, project, and the read-only continue inspector.

use std::collections::BTreeMap;

use crewmux_core::Timestamp;
use crewmux_messaging::{Actor, NewItem, QueueBook, QueueState, SessionRef, Update};
use crewmux_spec::workflow::WorkflowSpec;
use crewmux_types::{QItemId, WorkflowInstanceId};
use sha2::{Digest, Sha256};

use crate::deadline::evaluate_deadline;
use crate::error::Error;
use crate::exit::Exit;
use crate::keepalive;
use crate::model::{CachedSpec, Engine, Instance, Status};
use crate::packet;
use crate::project::{self, ProjectCtx, ProjectRequest, Projection};
use crate::route::{reconcile_explicit, resolve_owner};
use crate::time::hex_encode;
use crate::validate::{self, Probes, Validation};
use crate::world::World;

/// Result of instantiate.
#[derive(Clone, Debug)]
pub struct Instantiate {
    /// New instance id.
    pub instance_id: String,
    /// Entry packet.
    pub entry_qitem: String,
    /// Entry owner.
    pub entry_owner: String,
    /// Loud notices. Empty when there is nothing to say.
    pub advisories: Vec<String>,
    /// True only for an operation-key replay. This slice always returns false.
    pub replayed: bool,
}

/// Read-only continue / trace view.
#[derive(Clone, Debug)]
pub struct ContinueView {
    /// Instance id.
    pub instance_id: String,
    /// Trail length.
    pub trail_len: usize,
}

/// Stateless entry points.
pub struct Runtime;

impl Runtime {
    /// Parse and validate a spec file the caller already read.
    ///
    /// # Errors
    ///
    /// Returns the parser's code and message.
    pub fn validate_text(text: &str, path: &str) -> Result<Validation, Error> {
        let spec = crewmux_spec::parse_workflow(text, path)
            .map_err(|err| Error::reject(err.code, err.message, 400))?;
        Ok(validate::validate(
            &spec,
            &Probes {
                liveness: None,
                hosts: None,
            },
        ))
    }

    /// Cache `text` and open the entry packet.
    ///
    /// # Errors
    ///
    /// Returns a spec, owner, or rig rejection before the draft is returned.
    #[expect(
        clippy::too_many_lines,
        reason = "instantiate validates, binds the rig, and writes the entry packet in one draft"
    )]
    pub fn instantiate(
        engine: &Engine,
        book: &QueueBook,
        request: &InstantiateRequest<'_>,
        world: &World,
        now: Timestamp,
    ) -> Result<(Engine, QueueBook, Instantiate), Error> {
        let spec = crewmux_spec::parse_workflow(request.yaml, request.path)
            .map_err(|err| Error::reject(err.code, err.message, 400))?;
        let report = validate::validate(
            &spec,
            &Probes {
                liveness: None,
                hosts: None,
            },
        );
        if !report.ok {
            let first = report
                .issues
                .iter()
                .find(|issue| matches!(issue.severity, validate::Severity::Error));
            let detail = first
                .map(|issue| format!("{}: {}", issue.code, issue.message))
                .unwrap_or_default();
            return Err(Error::reject(
                "spec_invalid",
                format!(
                    "workflow spec {}@{} is invalid. {detail}",
                    spec.id, spec.version_label
                ),
                400,
            ));
        }
        let bound = bind_rig(&spec, request.rig, world, request.path)?;
        let mut advisories = default_rig_advisory(&spec, request.rig, world);
        advisories.extend(member_advisories(&spec, world));
        let entry = spec.steps.first().ok_or_else(|| {
            Error::reject(
                "spec_no_steps",
                "workflow spec has no steps".to_owned(),
                400,
            )
        })?;
        let owner = entry_owner(&spec, entry, request.entry_owner, world, bound.as_ref())?;
        let mut draft_engine = engine.clone();
        let mut draft_book = book.clone();
        let instance_id = WorkflowInstanceId::generate();
        let qitem = QItemId::generate();
        let hash = hex_encode(Sha256::digest(request.yaml.as_bytes()).as_slice());
        let cached = CachedSpec {
            spec_id: WorkflowInstanceId::generate().to_string(),
            name: spec.id.clone(),
            version: spec.version_label.clone(),
            yaml: request.yaml.to_owned(),
            source_path: request.path.to_owned(),
            source_hash: hash,
            cached_at: now,
            built_in: false,
        };
        draft_engine
            .specs
            .insert(Engine::spec_key(&spec.id, &spec.version_label), cached);
        let body = packet::entry_body(
            &spec,
            instance_id.as_str(),
            entry,
            request.objective,
            qitem.as_str(),
            &owner,
        );
        draft_book.create(
            NewItem {
                id: Some(qitem.clone()),
                source: session(request.created_by)?,
                destination: session(&owner)?,
                body,
                priority: 0,
                tier: Some("mode2".to_owned()),
                tags: vec![
                    "workflow".to_owned(),
                    "entry".to_owned(),
                    format!("workflow:{}", spec.id),
                    format!("instance:{}", instance_id),
                ],
                handed_off_from: None,
                last_nudge: None,
                explicit_human: false,
                fingerprint: String::new(),
            },
            now,
        )?;
        draft_book.append_note(
            &qitem,
            "workflow.nudge",
            Some("workflow nudge"),
            Some("recorded"),
            now,
        )?;
        draft_engine.instances.push(Instance {
            id: instance_id.clone(),
            workflow_name: spec.id.clone(),
            workflow_version: spec.version_label.clone(),
            created_by: session(request.created_by)?,
            created_at: now,
            status: Status::Active,
            frontier: vec![qitem.clone()],
            current_step_id: Some(entry.id.clone()),
            hop_count: 0,
            fallback_synthesis: None,
            last_decision: None,
            completed_at: None,
            version: 1,
            resume_count: 0,
            hops_baseline: 0,
            bound_rig: bound,
        });
        keepalive::arm(&mut draft_engine, instance_id.as_str(), &owner);
        Ok((
            draft_engine,
            draft_book,
            Instantiate {
                instance_id: instance_id.to_string(),
                entry_qitem: qitem.to_string(),
                entry_owner: owner,
                advisories,
                replayed: false,
            },
        ))
    }

    /// Advance one exit. `continue` does not call this.
    ///
    /// # Errors
    ///
    /// Returns the projector error. A crash leaves the inputs unchanged.
    pub fn project(
        engine: &Engine,
        book: &QueueBook,
        request: &ProjectRequest,
        ctx: &mut ProjectCtx<'_>,
    ) -> Result<(Engine, QueueBook, Projection), Error> {
        project::project_step(engine, book, request, ctx)
    }

    /// Read-only inspector. Two calls write nothing.
    ///
    /// # Errors
    ///
    /// Returns `instance_not_found`.
    pub fn continue_view(engine: &Engine, instance_id: &str) -> Result<ContinueView, Error> {
        let instance = engine.instance(instance_id).ok_or_else(|| {
            Error::reject(
                "instance_not_found",
                format!("workflow instance {instance_id} not found"),
                404,
            )
        })?;
        let trail_len = engine
            .trails
            .iter()
            .filter(|row| row.instance_id == instance.id.as_str())
            .count();
        Ok(ContinueView {
            instance_id: instance.id.as_str().to_owned(),
            trail_len,
        })
    }

    /// Abort a live instance. The frontier packet is canceled in the same draft.
    ///
    /// # Errors
    ///
    /// Returns `instance_not_abortable` when the instance is already terminal.
    pub fn abort(
        engine: &Engine,
        book: &QueueBook,
        instance_id: &str,
        actor: &str,
        reason: &str,
        now: Timestamp,
    ) -> Result<(Engine, QueueBook), Error> {
        let instance = live(engine, instance_id, "instance_not_abortable")?;
        let mut draft_engine = engine.clone();
        let mut draft_book = book.clone();
        for id in instance.frontier.clone() {
            draft_book.update(
                &id,
                Update {
                    state: Some(QueueState::Canceled),
                    actor: Actor::Session(session(actor)?),
                    note: format!("workflow abort: {reason}"),
                    closure_reason: Some("superseded".to_owned()),
                    closure_target: Some(reason.to_owned()),
                    reopen: false,
                    blocked_on: None,
                },
                now,
            )?;
        }
        let row = draft_engine
            .instance_mut(instance_id)
            .ok_or_else(|| missing(instance_id))?;
        row.status = Status::Aborted;
        row.frontier.clear();
        row.current_step_id = None;
        row.completed_at = Some(now);
        row.version = row.version.saturating_add(1);
        keepalive::disarm(&mut draft_engine, instance_id, "workflow_aborted");
        Ok((draft_engine, draft_book))
    }

    /// Point the frontier packet at another owner without advancing the step.
    ///
    /// # Errors
    ///
    /// Returns a frontier or session rejection.
    pub fn route(
        engine: &Engine,
        book: &QueueBook,
        instance_id: &str,
        to: &str,
        actor: &str,
        now: Timestamp,
    ) -> Result<(Engine, QueueBook, String), Error> {
        let instance = live(engine, instance_id, "instance_not_active")?;
        let current = instance.frontier.first().cloned().ok_or_else(|| {
            Error::reject(
                "packet_not_on_frontier",
                "workflow frontier is empty".to_owned(),
                409,
            )
        })?;
        let mut draft_engine = engine.clone();
        let mut draft_book = book.clone();
        let destination = session(to)?;
        draft_book.fallback(&current, &destination, &session(actor)?, now)?;
        keepalive::arm(&mut draft_engine, instance_id, to);
        if let Some(row) = draft_engine.instance_mut(instance_id) {
            row.version = row.version.saturating_add(1);
        }
        Ok((draft_engine, draft_book, current.to_string()))
    }

    /// Open a fresh packet for a failed or aborted instance. This is not `project`.
    ///
    /// # Errors
    ///
    /// Returns `instance_not_resumable` unless the status is failed or aborted.
    pub fn resume(
        engine: &Engine,
        book: &QueueBook,
        instance_id: &str,
        actor: &str,
        owner: &str,
        now: Timestamp,
    ) -> Result<(Engine, QueueBook, String), Error> {
        let instance = engine
            .instance(instance_id)
            .ok_or_else(|| missing(instance_id))?;
        if !matches!(instance.status, Status::Failed | Status::Aborted) {
            return Err(Error::reject(
                "instance_not_resumable",
                format!("workflow instance {instance_id} is not failed or aborted"),
                409,
            ));
        }
        let step = instance
            .current_step_id
            .clone()
            .unwrap_or_else(|| "entry".to_owned());
        let mut draft_engine = engine.clone();
        let mut draft_book = book.clone();
        let qitem = QItemId::generate();
        draft_book.create(
            NewItem {
                id: Some(qitem.clone()),
                source: session(actor)?,
                destination: session(owner)?,
                body: format!("workflow resume {instance_id} step {step}"),
                priority: 0,
                tier: Some("mode2".to_owned()),
                tags: vec![
                    "workflow".to_owned(),
                    "resume".to_owned(),
                    format!("instance:{instance_id}"),
                ],
                handed_off_from: None,
                last_nudge: None,
                explicit_human: false,
                fingerprint: String::new(),
            },
            now,
        )?;
        let row = draft_engine
            .instance_mut(instance_id)
            .ok_or_else(|| missing(instance_id))?;
        let hops = row.hop_count;
        row.status = Status::Active;
        row.frontier = vec![qitem.clone()];
        row.current_step_id = Some(step);
        row.completed_at = None;
        row.resume_count = row.resume_count.saturating_add(1);
        row.hops_baseline = hops;
        row.version = row.version.saturating_add(1);
        keepalive::arm(&mut draft_engine, instance_id, owner);
        Ok((draft_engine, draft_book, qitem.to_string()))
    }
}

/// Instantiate inputs. The YAML is already loaded off the worker thread.
pub struct InstantiateRequest<'a> {
    /// Spec YAML.
    pub yaml: &'a str,
    /// Path quoted in errors and stored on the cache row.
    pub path: &'a str,
    /// Root objective.
    pub objective: &'a str,
    /// Creator session.
    pub created_by: &'a str,
    /// Explicit entry owner.
    pub entry_owner: Option<&'a str>,
    /// Explicit `--rig`.
    pub rig: Option<&'a str>,
}

fn bind_rig(
    spec: &WorkflowSpec,
    explicit: Option<&str>,
    world: &World,
    path: &str,
) -> Result<Option<String>, Error> {
    if let Some(rig) = explicit {
        if !world.rigs.contains(rig) {
            let registered = if world.rigs.is_empty() {
                "(none)".to_owned()
            } else {
                world.rigs.iter().cloned().collect::<Vec<_>>().join(", ")
            };
            return Err(Error::reject(
                "bound_rig_unknown",
                format!(
                    "cannot instantiate: target rig \"{rig}\" is not a registered rig on this daemon. Registered rigs: {registered}. Check `rig ps`, create/import the rig first, or instantiate with a different --rig."
                ),
                400,
            ));
        }
        return Ok(Some(rig.to_owned()));
    }
    if let Some(rig) = spec.target_rig.as_deref()
        && !world.rigs.contains(rig)
    {
        let _ = path;
        return Ok(None);
    }
    Ok(spec.target_rig.clone())
}

/// Advisory for an unknown spec-default rig. The caller appends member advisories.
#[must_use]
pub(crate) fn default_rig_advisory(
    spec: &WorkflowSpec,
    explicit: Option<&str>,
    world: &World,
) -> Vec<String> {
    if explicit.is_some() {
        return Vec::new();
    }
    let Some(rig) = spec.target_rig.as_deref() else {
        return Vec::new();
    };
    if world.rigs.contains(rig) {
        return Vec::new();
    }
    let registered = if world.rigs.is_empty() {
        "(none)".to_owned()
    } else {
        world.rigs.iter().cloned().collect::<Vec<_>>().join(", ")
    };
    vec![format!(
        "workflow spec default target.rig \"{rig}\" is not a registered rig on this daemon — instantiating UNBOUND. Steps route via their declared preferred_targets; any role-only step (no preferred_targets) will fail per-role at the right time (entry at instantiate, later steps at projection). Pass --rig <name> to bind explicitly. Registered rigs: {registered}."
    )]
}

fn member_advisories(spec: &WorkflowSpec, world: &World) -> Vec<String> {
    let mut order = Vec::new();
    let mut declared: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for step in &spec.steps {
        note_role(
            spec,
            world,
            &step.id,
            &step.actor_role,
            &mut order,
            &mut declared,
        );
        if let Some(gate) = &step.gate
            && !crewmux_messaging::is_human_seat(&gate.target)
        {
            note_role(
                spec,
                world,
                &step.id,
                &gate.target,
                &mut order,
                &mut declared,
            );
        }
    }
    order
        .into_iter()
        .filter_map(|target| {
            let steps = declared.remove(&target)?;
            Some(advisory_line(&target, &steps))
        })
        .collect()
}

fn note_role(
    spec: &WorkflowSpec,
    world: &World,
    step_id: &str,
    role_name: &str,
    order: &mut Vec<String>,
    declared: &mut BTreeMap<String, Vec<String>>,
) {
    let Some(role) = spec.roles.get(role_name) else {
        return;
    };
    for target in &role.preferred_targets {
        if !missing_member(target, world) {
            continue;
        }
        if !declared.contains_key(target) {
            order.push(target.clone());
        }
        declared
            .entry(target.clone())
            .or_default()
            .push(format!("step \"{step_id}\" (role \"{role_name}\")"));
    }
}

fn missing_member(target: &str, world: &World) -> bool {
    if crewmux_messaging::is_human_seat(target) {
        return false;
    }
    let Some((member, rig)) = target.split_once('@') else {
        return false;
    };
    !member.is_empty()
        && !rig.is_empty()
        && world.rigs.contains(rig)
        && !world.members.contains(target)
}

fn advisory_line(target: &str, steps: &[String]) -> String {
    let rig = target.split_once('@').map_or("", |(_, rig)| rig);
    format!(
        "preferred target \"{target}\" names rig \"{rig}\" (registered) but NO member of that rig has this coordinate — declared by {}. Work routed there will not be claimed; it will surface as a stuck exception. Check the member name against `rig ps`, or add the member to rig \"{rig}\".",
        steps.join(", ")
    )
}

fn entry_owner(
    spec: &WorkflowSpec,
    step: &crewmux_spec::workflow::StepSpec,
    explicit: Option<&str>,
    world: &World,
    bound: Option<&String>,
) -> Result<String, Error> {
    if let Some(owner) = explicit {
        reconcile_explicit(step, owner, world).map_err(rename_entry)?;
        return Ok(owner.to_owned());
    }
    match resolve_owner(spec, step, world, bound.is_some()).map_err(rename_entry)? {
        Some(owner) => Ok(owner),
        None => Err(Error::reject(
            "entry_owner_unresolved",
            format!(
                "cannot instantiate: entry step \"{}\" (role \"{}\") has no preferred_targets and no entryOwnerSession was supplied",
                step.id, step.actor_role
            ),
            400,
        )),
    }
}

fn rename_entry(err: Error) -> Error {
    match err {
        Error::Rejected {
            code: "next_owner_unresolved",
            message,
            status: _,
        } => Error::reject(
            "entry_owner_unresolved",
            format!("cannot instantiate: {message}"),
            400,
        ),
        Error::Rejected {
            code,
            message,
            status,
        } => Error::Rejected {
            code,
            message,
            status,
        },
        Error::Crashed(point) => Error::Crashed(point),
    }
}

fn live<'a>(engine: &'a Engine, id: &str, code: &'static str) -> Result<&'a Instance, Error> {
    let instance = engine.instance(id).ok_or_else(|| missing(id))?;
    if !instance.status.is_live() {
        return Err(Error::reject(
            code,
            format!(
                "workflow instance {id} has status {}",
                instance.status.as_str()
            ),
            409,
        ));
    }
    Ok(instance)
}

fn missing(id: &str) -> Error {
    Error::reject(
        "instance_not_found",
        format!("workflow instance {id} not found"),
        404,
    )
}

fn session(text: &str) -> Result<SessionRef, Error> {
    SessionRef::new(text).map_err(Error::from)
}

/// Deadline for a read surface.
#[must_use]
pub fn deadline_for(
    engine: &Engine,
    book: &QueueBook,
    instance_id: &str,
    now: Timestamp,
) -> Option<crate::deadline::Verdict> {
    let instance = engine.instance(instance_id)?;
    let packets = instance
        .frontier
        .iter()
        .filter_map(|id| book.get(id).cloned())
        .collect::<Vec<_>>();
    Some(evaluate_deadline(instance, &packets, now))
}

/// Guidance lines for the current step. Read-only.
#[must_use]
pub fn guidance_lines(engine: &Engine, instance_id: &str) -> Vec<String> {
    let Some(instance) = engine.instance(instance_id) else {
        return vec![format!("workflow instance {instance_id} not found")];
    };
    let step = instance.current_step_id.as_deref().unwrap_or("(none)");
    let packet = instance.frontier.first().map_or("(none)", QItemId::as_str);
    vec![
        format!(
            "Workflow {}@{} step {step}",
            instance.workflow_name, instance.workflow_version
        ),
        format!("Frontier packet {packet}. project is the only advance."),
    ]
}

/// Exit parser used by the CLI before it posts.
#[must_use]
pub fn parse_exit(text: &str) -> Option<Exit> {
    Exit::parse(text)
}
