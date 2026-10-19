// Derived from openrig packages/daemon/src/domain/policies/workflow-keepalive.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Paste bodies. They match the TypeScript strings byte for byte.

use sha2::{Digest, Sha256};

use crate::deadline::Evidence;
use crate::time::{hex_encode, iso8601};

/// Trailer on an operator keepalive. Deadline nudges do not use it.
pub(crate) const KEEPALIVE_TRAILER: &str = "Continue the current step. If the run has ended, update the workflow state honestly and manufacture the next truthful packet now. If you think you need approval, first ask whether there is real product ambiguity or only approval theater. Keep communications bridged, name exact blockers, and self-scout deterministic bias: add only the minimum deterministic code required for reliable agent operation, then rely on agent judgment for routing, adaptation, and edge handling.";

/// Happy-path keepalive. `others` is every session except the one addressed.
#[must_use]
pub fn keepalive_message(
    workflow_name: &str,
    workflow_version: &str,
    instance_id: &str,
    status: &str,
    others: &[String],
) -> String {
    let mut lines = vec![
        format!(
            "Workflow keepalive: {workflow_name}@{workflow_version} / {instance_id} is still live (status: {status})."
        ),
        KEEPALIVE_TRAILER.to_owned(),
    ];
    if !others.is_empty() {
        lines.push(String::new());
        lines.push(format!(
            "Other frontier owners + observers: {}",
            others.join(", ")
        ));
    }
    lines.join("\n")
}

/// Overdue re-nudge. Names the packet, the anchor, and the inspect commands.
#[must_use]
pub fn stuck_message(evidence: &Evidence) -> String {
    let minutes = evidence.overdue_by_seconds.div_euclid(60);
    let anchor_at = iso8601(evidence.anchor_at);
    format!(
        "Workflow deadline: {instance}, packet {packet}, owner {owner}; {minutes}m overdue ({anchor} at {anchor_at}).\nPacket age does not establish idle. Inspect: rig workflow show {instance}. Full packet: rig queue show {packet} --full.",
        instance = evidence.instance_id,
        packet = evidence.packet_id,
        owner = evidence.owner_session,
        anchor = evidence.anchor,
    )
}

/// `sha256` of the TypeScript `JSON.stringify([jobId, generation, receipt])`.
#[must_use]
pub fn occurrence_id(job_id: &str, generation: Option<&str>, receipt: &str) -> String {
    let generation = generation.map_or(serde_json::Value::Null, |text| {
        serde_json::Value::String(text.to_owned())
    });
    let payload = serde_json::json!([job_id, generation, receipt]);
    let bytes = serde_json::to_vec(&payload).unwrap_or_default();
    hex_encode(Sha256::digest(&bytes).as_slice())
}
