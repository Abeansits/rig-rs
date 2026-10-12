// Derived from openrig packages/daemon/src/domain/health-projection.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! One finding. Identity is a hash of the detector, subject, and start time.

use crewmux_core::Timestamp;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::coverage::Coverage;
use crate::detector::DetectorId;
use crate::time::iso_timestamp;

/// List document schema.
pub const HEALTH_LIST_SCHEMA: &str = "openrig.health-list/v0alpha1";
/// Record schema.
pub const HEALTH_RECORD_SCHEMA: &str = "openrig.health/v0alpha1";

/// `info`, `warning`, or `critical`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Informational.
    Info,
    /// Warning.
    Warning,
    /// Critical.
    Critical,
}

/// `active`, `cleared`, or `indeterminate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// The condition holds.
    Active,
    /// The condition cleared.
    Cleared,
    /// The census cannot decide.
    Indeterminate,
}

/// A projected finding.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthRecord {
    /// Schema token.
    pub schema: &'static str,
    /// `health-` plus 24 hex chars.
    pub id: String,
    /// Detector that produced the finding.
    pub detector: DetectorId,
    /// `process`, `governance`, or `context`.
    pub category: &'static str,
    /// How serious the finding is.
    pub severity: Severity,
    /// Whether it is still open.
    pub status: Status,
    /// Seat, lineage, directive, or slice.
    pub subject: String,
    /// One-line summary.
    pub summary: String,
    /// Scope object. Field names match the TypeScript record.
    pub scope: Value,
    /// Evidence refs.
    pub evidence: Vec<Value>,
    /// The rule that fired, as text.
    pub threshold: String,
    /// Why the rule fired.
    pub explanation: String,
    /// Suggested next look.
    pub suggested_inspection: String,
    /// Coverage of the source that produced this finding, when the source bounded its read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage: Option<Coverage>,
}

/// Stable id. The same detector, subject, and start time keep the same id.
#[must_use]
pub fn finding_id(detector: DetectorId, subject: &str, at: Timestamp) -> String {
    let mut hasher = Sha256::new();
    hasher.update(detector.as_str().as_bytes());
    hasher.update([0]);
    hasher.update(subject.as_bytes());
    hasher.update([0]);
    hasher.update(iso_timestamp(at).as_bytes());
    let digest = hasher.finalize();
    let mut id = String::from("health-");
    for byte in digest.iter().take(12) {
        push_hex(&mut id, *byte);
    }
    id
}

fn push_hex(out: &mut String, byte: u8) {
    const HEX: &str = "0123456789abcdef";
    let hi = usize::from(byte >> 4);
    let lo = usize::from(byte & 0x0f);
    if let Some(ch) = HEX.chars().nth(hi) {
        out.push(ch);
    }
    if let Some(ch) = HEX.chars().nth(lo) {
        out.push(ch);
    }
}

pub(crate) fn seat_scope(seat: &str) -> Value {
    serde_json::json!({"type": "seat", "rigId": "stub", "seatId": seat})
}

pub(crate) fn instance_scope(id: &str) -> Value {
    serde_json::json!({"type": "instance", "instanceId": id})
}
