// Derived from openrig packages/daemon/src/domain/workflow-runtime.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Read-only `inspect`. Show and continue report this view.

use crewmux_core::Timestamp;
use crewmux_messaging::QueueBook;
use crewmux_spec::workflow::{Acceptance, GateSpec, StepSpec, WorkflowSpec};
use crewmux_types::QItemId;
use serde_json::{Value, json};

use crate::deadline::evaluate_deadline;
use crate::error::Error;
use crate::model::{Engine, Instance};
use crate::wire::verdict_json;

/// Frontier packets, failure occurrences, unknowns, and boundary obligations.
#[derive(Clone, Debug, PartialEq)]
pub struct Inspected {
    /// One object per live frontier packet.
    pub frontier: Vec<Value>,
    /// Stored failure occurrences. This engine does not keep that table.
    pub failures: Vec<Value>,
    /// Frontier packets that have no step binding or no queue row.
    pub unknowns: Vec<String>,
    /// Lifecycle obligations. Empty without a lifecycle binding, as in TypeScript.
    pub boundary_obligations: Vec<Value>,
}

/// Inspect `instance_id`. A missing instance is `instance_not_found`.
///
/// # Errors
///
/// Returns [`Error::Rejected`] when the instance is absent.
pub fn inspect(
    engine: &Engine,
    book: &QueueBook,
    instance_id: &str,
    now: Timestamp,
) -> Result<Inspected, Error> {
    let instance = engine.instance(instance_id).ok_or_else(|| {
        Error::reject(
            "instance_not_found",
            format!("workflow instance {instance_id} not found"),
            404,
        )
    })?;
    let spec = engine
        .specs
        .get(&Engine::spec_key(
            &instance.workflow_name,
            &instance.workflow_version,
        ))
        .and_then(|cached| crewmux_spec::parse_workflow(&cached.yaml, &cached.source_path).ok());
    let mut unknowns = Vec::new();
    let mut frontier = Vec::new();
    for packet_id in &instance.frontier {
        frontier.push(one_packet(
            instance,
            book,
            spec.as_ref(),
            packet_id,
            &mut unknowns,
            now,
        ));
    }
    Ok(Inspected {
        frontier,
        failures: Vec::new(),
        unknowns,
        boundary_obligations: Vec::new(),
    })
}

fn one_packet(
    instance: &Instance,
    book: &QueueBook,
    spec: Option<&WorkflowSpec>,
    packet_id: &QItemId,
    unknowns: &mut Vec<String>,
    now: Timestamp,
) -> Value {
    let step_id = single_step(instance, packet_id, unknowns);
    let packet = book.get(packet_id);
    if packet.is_none() {
        unknowns.push(format!("frontier packet {packet_id} has no queue row"));
    }
    let step = step_id.as_deref().and_then(|id| find_step(spec, id));
    let action = if step_id.is_some() && packet.is_some() {
        "project"
    } else {
        "indeterminate"
    };
    let mut view = instance.clone();
    view.frontier = vec![packet_id.clone()];
    view.current_step_id.clone_from(&step_id);
    let packets = packet.cloned().into_iter().collect::<Vec<_>>();
    let deadline = evaluate_deadline(&view, &packets, now);
    json!({
        "packetId": packet_id.as_str(),
        "stepId": step_id,
        "ownerSession": packet.map(|item| item.destination.as_str()),
        "queueState": packet.map(|item| item.state.as_str()),
        "blockedOn": packet.and_then(|item| item.blocked_on.as_ref().map(|seat| seat.as_str().to_owned())),
        "targetedAction": action,
        "dependsOn": step.and_then(|step| step.depends_on.clone()).unwrap_or_default(),
        "gate": step.and_then(|step| step.gate.as_ref()).map(gate_json),
        "acceptance": step.and_then(|step| step.acceptance.as_ref()).map(acceptance_json),
        "receiptRequired": false,
        "deadline": verdict_json(&deadline),
        "waiting": Value::Null,
    })
}

fn single_step(
    instance: &Instance,
    packet_id: &QItemId,
    unknowns: &mut Vec<String>,
) -> Option<String> {
    if instance.frontier.len() == 1 {
        return instance.current_step_id.clone();
    }
    unknowns.push(format!(
        "frontier packet {packet_id} has no unique step binding"
    ));
    None
}

fn find_step<'a>(spec: Option<&'a WorkflowSpec>, id: &str) -> Option<&'a StepSpec> {
    spec.and_then(|spec| spec.steps.iter().find(|step| step.id == id))
}

fn gate_json(gate: &GateSpec) -> Value {
    let mut body = serde_json::Map::new();
    body.insert("target".to_owned(), json!(gate.target));
    if let Some(summary) = &gate.summary {
        body.insert("summary".to_owned(), json!(summary));
    }
    if let Some(evidence) = &gate.evidence_ref {
        body.insert("evidence_ref".to_owned(), json!(evidence));
    }
    Value::Object(body)
}

fn acceptance_json(acceptance: &Acceptance) -> Value {
    json!({
        "candidate": acceptance.candidate,
        "verdicts": acceptance.verdicts,
        "evidence_ref": acceptance.evidence_ref,
    })
}
