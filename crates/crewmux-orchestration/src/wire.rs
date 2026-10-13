//! JSON shapes the daemon returns. Key order is not significant; the diff harness sorts it.

use crewmux_spec::workflow::{RoleSpec, StepSpec, WorkflowSpec};
use crewmux_types::QItemId;
use serde_json::{Map, Value, json};

use crate::deadline::{Evidence, Verdict};
use crate::model::{CachedSpec, Decision, Instance, Trail};
use crate::project::Projection;
use crate::time::iso8601;
use crate::validate::Validation;

/// Instance object. `deadline` is attached on read surfaces.
#[must_use]
pub fn instance_json(instance: &Instance, deadline: Option<&Verdict>) -> Value {
    let mut body = Map::new();
    body.insert("instanceId".to_owned(), json!(instance.id.as_str()));
    body.insert("workflowName".to_owned(), json!(instance.workflow_name));
    body.insert(
        "workflowVersion".to_owned(),
        json!(instance.workflow_version),
    );
    body.insert(
        "createdBySession".to_owned(),
        json!(instance.created_by.as_str()),
    );
    body.insert("createdAt".to_owned(), json!(iso8601(instance.created_at)));
    body.insert("status".to_owned(), json!(instance.status.as_str()));
    body.insert(
        "currentFrontier".to_owned(),
        json!(
            instance
                .frontier
                .iter()
                .map(QItemId::as_str)
                .collect::<Vec<_>>()
        ),
    );
    body.insert("currentStepId".to_owned(), json!(instance.current_step_id));
    body.insert("hopCount".to_owned(), json!(instance.hop_count));
    body.insert("fallbackSynthesis".to_owned(), Value::Null);
    body.insert(
        "lastContinuationDecision".to_owned(),
        instance
            .last_decision
            .as_ref()
            .map_or(Value::Null, decision_json),
    );
    body.insert(
        "completedAt".to_owned(),
        instance
            .completed_at
            .map(iso8601)
            .map_or(Value::Null, Value::String),
    );
    body.insert("version".to_owned(), json!(instance.version));
    body.insert("resumeCount".to_owned(), json!(instance.resume_count));
    body.insert("hopsBaseline".to_owned(), json!(instance.hops_baseline));
    body.insert("boundRig".to_owned(), json!(instance.bound_rig));
    body.insert("lifecycleOperationKey".to_owned(), Value::Null);
    body.insert("compiledInputDigest".to_owned(), Value::Null);
    body.insert("lifecycleBinding".to_owned(), Value::Null);
    if let Some(deadline) = deadline {
        body.insert("deadline".to_owned(), verdict_json(deadline));
    }
    Value::Object(body)
}

/// Validation report.
#[must_use]
pub fn validation_json(report: &Validation) -> Value {
    serde_json::to_value(report).unwrap_or(json!({"ok": false, "issues": [], "summary": {}}))
}

/// Cache row. The nested spec carries `hot_potato` when the author omitted it.
#[must_use]
pub fn spec_row_json(cached: &CachedSpec, spec: &WorkflowSpec) -> Value {
    let mut row = spec_row_body(cached, spec);
    if let Value::Object(body) = &mut row {
        body.insert("isBuiltIn".to_owned(), json!(cached.built_in));
    }
    row
}

/// Instantiate's spec row. TypeScript's `WorkflowSpecRow` has no `isBuiltIn`.
#[must_use]
pub fn instantiate_spec_json(cached: &CachedSpec, spec: &WorkflowSpec) -> Value {
    spec_row_body(cached, spec)
}

fn spec_row_body(cached: &CachedSpec, spec: &WorkflowSpec) -> Value {
    let rule = spec
        .coordination_terminal_turn_rule
        .clone()
        .unwrap_or_else(|| "hot_potato".to_owned());
    json!({
        "specId": cached.spec_id,
        "name": cached.name,
        "version": cached.version,
        "purpose": spec.objective,
        "targetRig": spec.target_rig,
        "spec": spec_json(spec),
        "coordinationTerminalTurnRule": rule,
        "sourcePath": cached.source_path,
        "sourceHash": cached.source_hash,
        "cachedAt": iso8601(cached.cached_at),
    })
}

/// Project result. No deadline on the instance.
#[must_use]
pub fn project_json(instance: &Instance, projection: &Projection) -> Value {
    let mut body = Map::new();
    body.insert("instance".to_owned(), instance_json(instance, None));
    body.insert(
        "closurePriorPacketId".to_owned(),
        json!(projection.closure_prior),
    );
    body.insert("closureReason".to_owned(), json!(projection.closure_reason));
    body.insert("nextQitemId".to_owned(), json!(projection.next_qitem));
    body.insert("nextOwnerSession".to_owned(), json!(projection.next_owner));
    body.insert("nextStepId".to_owned(), json!(projection.next_step));
    body.insert(
        "emittedEventTypes".to_owned(),
        json!(
            projection
                .events
                .iter()
                .map(|event| (*event).to_owned())
                .collect::<Vec<_>>()
        ),
    );
    if projection.absorbed {
        body.insert("absorbedReplay".to_owned(), json!(true));
    }
    Value::Object(body)
}

/// One trail row.
#[must_use]
pub fn trail_json(row: &Trail) -> Value {
    json!({
        "trailId": row.id,
        "instanceId": row.instance_id,
        "stepId": row.step_id,
        "stepRole": row.step_role,
        "closedAt": iso8601(row.closed_at),
        "closureReason": row.closure_reason,
        "closureEvidence": row.closure_evidence,
        "actorSession": row.actor_session,
        "nextQitemId": row.next_qitem_id,
        "priorQitemId": row.prior_qitem_id,
    })
}

fn decision_json(decision: &Decision) -> Value {
    json!({
        "exit": decision.exit,
        "actorSession": decision.actor_session,
        "closedPacket": decision.closed_packet,
        "nextPacket": decision.next_packet,
        "resultNote": decision.result_note,
        "blockedOn": decision.blocked_on,
        "currentStep": decision.current_step,
        "branchTaken": decision.branch_taken,
    })
}

fn verdict_json(verdict: &Verdict) -> Value {
    json!({
        "state": verdict.state,
        "evidence": verdict.evidence.as_ref().map(evidence_json),
    })
}

fn evidence_json(evidence: &Evidence) -> Value {
    json!({
        "instanceId": evidence.instance_id,
        "stepId": evidence.step_id,
        "packetId": evidence.packet_id,
        "ownerSession": evidence.owner_session,
        "packetState": evidence.packet_state,
        "anchor": evidence.anchor,
        "anchorAt": iso8601(evidence.anchor_at),
        "overdueBySeconds": evidence.overdue_by_seconds,
        "ageSeconds": evidence.age_seconds,
        "claimedAt": evidence.claimed_at.map(iso8601),
    })
}

fn spec_json(spec: &WorkflowSpec) -> Value {
    let mut body = Map::new();
    body.insert("id".to_owned(), json!(spec.id));
    body.insert("version".to_owned(), json!(spec.version_label));
    insert_opt(&mut body, "objective", spec.objective.clone());
    if let Some(rig) = &spec.target_rig {
        body.insert("target".to_owned(), json!({"rig": rig}));
    }
    if let Some(role) = &spec.entry_role {
        body.insert("entry".to_owned(), json!({"role": role}));
    }
    body.insert("roles".to_owned(), roles_json(spec));
    body.insert(
        "steps".to_owned(),
        Value::Array(spec.steps.iter().map(step_json).collect()),
    );
    // The cache row fills `hot_potato` when the author omitted the key, and
    // that default is copied onto the nested spec (workflow-spec-cache.ts).
    let rule = spec
        .coordination_terminal_turn_rule
        .clone()
        .unwrap_or_else(|| "hot_potato".to_owned());
    body.insert("coordination_terminal_turn_rule".to_owned(), json!(rule));
    Value::Object(body)
}

fn roles_json(spec: &WorkflowSpec) -> Value {
    let mut roles = Map::new();
    let names = if spec.role_order.is_empty() {
        spec.roles.keys().cloned().collect::<Vec<_>>()
    } else {
        spec.role_order.clone()
    };
    for name in names {
        if let Some(role) = spec.roles.get(&name) {
            roles.insert(name, role_json(role));
        }
    }
    Value::Object(roles)
}

fn role_json(role: &RoleSpec) -> Value {
    let mut body = Map::new();
    if !role.skill_refs.is_empty() {
        body.insert("skill_refs".to_owned(), json!(role.skill_refs));
    }
    if !role.preferred_targets.is_empty() {
        body.insert(
            "preferred_targets".to_owned(),
            json!(role.preferred_targets),
        );
    }
    Value::Object(body)
}

fn step_json(step: &StepSpec) -> Value {
    let mut body = Map::new();
    body.insert("id".to_owned(), json!(step.id));
    body.insert("actor_role".to_owned(), json!(step.actor_role));
    insert_opt(&mut body, "objective", step.objective.clone());
    if let Some(exits) = &step.allowed_exits {
        body.insert("allowed_exits".to_owned(), json!(exits));
    }
    if let Some(hop) = &step.next_hop {
        let mut on = Map::new();
        for (exit, target) in &hop.on {
            on.insert(exit.clone(), json!(target));
        }
        let mut next = Map::new();
        insert_opt(&mut next, "mode", hop.mode.clone());
        if !hop.suggested_roles.is_empty() {
            next.insert("suggested_roles".to_owned(), json!(hop.suggested_roles));
        }
        if !on.is_empty() {
            next.insert("on".to_owned(), Value::Object(on));
        }
        body.insert("next_hop".to_owned(), Value::Object(next));
    }
    insert_opt(&mut body, "harness", step.harness.clone());
    insert_opt(&mut body, "host", step.host.clone());
    Value::Object(body)
}

fn insert_opt(body: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        body.insert(key.to_owned(), json!(value));
    }
}
