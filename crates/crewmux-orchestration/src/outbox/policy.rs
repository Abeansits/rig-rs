// Derived from openrig packages/daemon/src/domain/policies/workflow-keepalive.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Keepalive decision against the workflow document and the queue book.

use crewmux_core::Timestamp;
use crewmux_messaging::{QueueBook, QueueItem};
use serde_json::{Value, json};

use crate::deadline::{self, Evidence};
use crate::model::{Engine, Status};
use crate::time::iso8601;

use super::text::{keepalive_message, stuck_message};

/// What one evaluation decided.
#[derive(Clone, Debug, PartialEq)]
pub enum WatchAction {
    /// Paste `message` to `target`.
    Send {
        /// Seat address.
        target: String,
        /// Authored body. The delivery path wraps it; this is not the envelope.
        message: String,
        /// Stable overdue identity. Absent for an operator keepalive.
        condition: Option<String>,
        /// Audit notes. `additionalRoutingTargets` is the TS field.
        notes: Value,
    },
    /// Do not paste.
    Skip {
        /// TypeScript reason token.
        reason: &'static str,
    },
    /// The job will not become due again.
    Terminal {
        /// TypeScript reason token. Unknown policies use `unknown_policy:{name}`.
        reason: String,
        /// Instance id when one was named.
        notes: Value,
    },
}

/// The spec named no instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchFail;

impl std::fmt::Display for WatchFail {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("policy_spec_invalid")
    }
}

/// Inputs the policy reads. The scheduler has already parsed the spec.
pub struct KeepaliveJob<'a> {
    /// Job id. Carried for the caller; the decision does not paste it.
    pub job_id: &'a str,
    /// Registered target. Used when the frontier does not name an owner.
    pub target_session: &'a str,
    /// Operator message. Deadline-gated overdue text replaces it.
    pub message: Option<&'a str>,
    /// Required. Missing is [`WatchFail`].
    pub instance_id: Option<&'a str>,
    /// Optional packet selector.
    pub packet_id: Option<&'a str>,
    /// One extra observer.
    pub observer_session: Option<&'a str>,
    /// Extra observers.
    pub observer_sessions: &'a [String],
    /// Auto-armed jobs stay quiet while the deadline is healthy.
    pub deadline_gated: bool,
    /// Last overdue identity that was pasted.
    pub last_fired_condition: Option<&'a str>,
}

/// Reasons that do not append a history row.
#[must_use]
pub fn is_quiet(reason: &str) -> bool {
    matches!(
        reason,
        "not_due"
            | "workflow_healthy_deadline_gated"
            | "workflow_recovery_owns_notice"
            | "workflow_deadline_already_presented"
            | "active_wake_not_due"
            | "no_actionable_artifacts"
            | "queue_wait_already_presented"
    )
}

/// Decide a keepalive. Reads `engine` and `book` only.
///
/// # Errors
///
/// Returns [`WatchFail`] when `instance_id` is absent (`policy_spec_invalid`).
pub fn evaluate_keepalive(
    engine: &Engine,
    book: &QueueBook,
    job: &KeepaliveJob<'_>,
    now: Timestamp,
) -> Result<WatchAction, WatchFail> {
    let Some(instance_id) = job.instance_id.filter(|id| !id.is_empty()) else {
        return Err(WatchFail);
    };
    let Some(instance) = engine.instance(instance_id) else {
        return Ok(missing(instance_id));
    };
    if !matches!(instance.status, Status::Active | Status::Waiting) {
        return Ok(inactive(instance_id, instance.status.as_str()));
    }
    let whole = frontier_ids(instance);
    if let Some(packet_id) = job.packet_id.filter(|id| !id.is_empty())
        && !whole.iter().any(|id| id == packet_id)
    {
        return Ok(not_live(instance_id, packet_id));
    }
    let frontier = selected_frontier(job, &whole);
    let rows = frontier_rows(book, &frontier);
    let verdict = deadline::evaluate_deadline(instance, &rows, now);
    if job.deadline_gated && verdict.state == "healthy" {
        return Ok(WatchAction::Skip {
            reason: "workflow_healthy_deadline_gated",
        });
    }
    let sessions = sessions_for(instance.created_by.as_str(), job, &rows);
    if sessions.is_empty() {
        return Ok(WatchAction::Skip {
            reason: "empty_frontier",
        });
    }
    Ok(send_or_skip(job, instance, &frontier, &sessions, &verdict))
}

fn missing(instance_id: &str) -> WatchAction {
    WatchAction::Terminal {
        reason: "workflow_instance_missing".to_owned(),
        notes: json!({ "instanceId": instance_id }),
    }
}

fn inactive(instance_id: &str, status: &str) -> WatchAction {
    WatchAction::Terminal {
        reason: "workflow_not_active".to_owned(),
        notes: json!({ "instanceId": instance_id, "status": status }),
    }
}

fn not_live(instance_id: &str, packet_id: &str) -> WatchAction {
    WatchAction::Terminal {
        reason: "workflow_packet_not_live".to_owned(),
        notes: json!({ "instanceId": instance_id, "packetId": packet_id }),
    }
}

fn frontier_ids(instance: &crate::model::Instance) -> Vec<String> {
    instance
        .frontier
        .iter()
        .map(|id| id.as_str().to_owned())
        .collect()
}

fn selected_frontier(job: &KeepaliveJob<'_>, whole: &[String]) -> Vec<String> {
    match job.packet_id.filter(|id| !id.is_empty()) {
        Some(packet_id) => vec![packet_id.to_owned()],
        None => whole.to_vec(),
    }
}

fn sessions_for(created_by: &str, job: &KeepaliveJob<'_>, rows: &[QueueItem]) -> Vec<String> {
    let owners: Vec<String> = rows
        .iter()
        .map(|row| row.destination.as_str().to_owned())
        .collect();
    dedup([owners, additional_targets(created_by, job)].concat())
}

fn send_or_skip(
    job: &KeepaliveJob<'_>,
    instance: &crate::model::Instance,
    frontier: &[String],
    sessions: &[String],
    verdict: &deadline::Verdict,
) -> WatchAction {
    let stuck = (job.deadline_gated && verdict.state != "healthy")
        .then_some(verdict.evidence.as_ref())
        .flatten();
    if let Some(evidence) = stuck {
        return stuck_send(job, instance, frontier, sessions, evidence, verdict.state);
    }
    operator_send(job, instance, frontier, sessions, verdict.state)
}

fn stuck_send(
    job: &KeepaliveJob<'_>,
    instance: &crate::model::Instance,
    frontier: &[String],
    sessions: &[String],
    evidence: &Evidence,
    verdict_state: &str,
) -> WatchAction {
    let condition = condition_receipt(evidence);
    if job.last_fired_condition == Some(condition.as_str()) {
        return WatchAction::Skip {
            reason: "workflow_deadline_already_presented",
        };
    }
    let primary = evidence.owner_session.clone();
    let others = sessions
        .iter()
        .filter(|session| session.as_str() != primary)
        .cloned()
        .collect::<Vec<_>>();
    WatchAction::Send {
        target: primary,
        message: stuck_message(evidence),
        condition: Some(condition),
        notes: send_notes(
            instance,
            frontier.len(),
            &others,
            Some(evidence),
            verdict_state,
        ),
    }
}

fn operator_send(
    job: &KeepaliveJob<'_>,
    instance: &crate::model::Instance,
    frontier: &[String],
    sessions: &[String],
    verdict_state: &str,
) -> WatchAction {
    let primary = sessions
        .first()
        .cloned()
        .unwrap_or_else(|| job.target_session.to_owned());
    let others = sessions.iter().skip(1).cloned().collect::<Vec<_>>();
    let message = match job.message {
        Some(text) => text.to_owned(),
        None => keepalive_message(
            &instance.workflow_name,
            &instance.workflow_version,
            instance.id.as_str(),
            instance.status.as_str(),
            &others,
        ),
    };
    WatchAction::Send {
        target: primary,
        message,
        condition: None,
        notes: send_notes(instance, frontier.len(), &others, None, verdict_state),
    }
}

fn frontier_rows(book: &QueueBook, frontier: &[String]) -> Vec<QueueItem> {
    frontier
        .iter()
        .filter_map(|id| {
            let parsed = crewmux_types::QItemId::new(id).ok()?;
            book.get(&parsed).cloned()
        })
        .collect()
}

fn additional_targets(created_by: &str, job: &KeepaliveJob<'_>) -> Vec<String> {
    let mut out = Vec::new();
    if !created_by.is_empty() {
        out.push(created_by.to_owned());
    }
    out.extend(job.observer_sessions.iter().cloned());
    if let Some(observer) = job.observer_session.filter(|text| !text.is_empty()) {
        out.push(observer.to_owned());
    }
    if !job.target_session.is_empty() {
        out.push(job.target_session.to_owned());
    }
    out
}

fn dedup(sessions: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for session in sessions {
        if !session.is_empty() && !out.iter().any(|have| have == &session) {
            out.push(session);
        }
    }
    out
}

fn condition_receipt(evidence: &Evidence) -> String {
    json!([
        evidence.packet_id,
        evidence.anchor,
        iso8601(evidence.anchor_at)
    ])
    .to_string()
}

fn send_notes(
    instance: &crate::model::Instance,
    frontier_len: usize,
    others: &[String],
    stuck: Option<&Evidence>,
    verdict_state: &str,
) -> Value {
    let mut notes = json!({
        "instanceId": instance.id.as_str(),
        "workflowName": instance.workflow_name,
        "workflowStatus": instance.status.as_str(),
        "frontierLength": frontier_len,
        "additionalRoutingTargets": others,
    });
    if let Some(evidence) = stuck
        && let Some(map) = notes.as_object_mut()
    {
        map.insert(
            "deadline".to_owned(),
            json!({
                "state": verdict_state,
                "packetId": evidence.packet_id,
                "anchor": evidence.anchor,
                "ownerSession": evidence.owner_session,
            }),
        );
    }
    notes
}
