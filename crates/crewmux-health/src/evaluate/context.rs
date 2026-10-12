// Derived from openrig packages/daemon/src/domain/health-detectors.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Context-pressure findings. A zero limit is not a zero-percent sample.

use crewmux_core::Timestamp;
use serde_json::json;

use super::Draft;
use crate::detector::DetectorId;
use crate::observation::HealthObservation;
use crate::policy::HealthPolicy;
use crate::record::{HealthRecord, Severity, Status, seat_scope};
use crate::summary::FRESHNESS_SECONDS;
use crate::time::iso_timestamp;

pub(super) fn context(
    observation: &HealthObservation,
    policy: &HealthPolicy,
    now: Timestamp,
) -> Vec<HealthRecord> {
    let HealthObservation::ContextPressure {
        seat,
        used_tokens,
        limit,
        at,
    } = observation
    else {
        return Vec::new();
    };
    if *limit == 0 {
        return Vec::new();
    }
    let pct = percent(*used_tokens, *limit);
    let warning = policy.thresholds.context_warning_percent;
    let critical = policy.thresholds.context_critical_percent;
    if pct < warning {
        return Vec::new();
    }
    let severity = if pct >= critical {
        Severity::Critical
    } else {
        Severity::Warning
    };
    let seat = seat.as_str();
    let fresh = now.saturating_duration_since(*at).as_secs() < FRESHNESS_SECONDS;
    vec![
        Draft {
            detector: DetectorId::ContextPressure,
            category: "context",
            severity,
            status: Status::Active,
            subject: seat.to_owned(),
            summary: format!("Seat context utilization reached {pct}%."),
            threshold: format!(
                "fresh context utilization >= {warning}% (critical at >= {critical}%)"
            ),
            explanation: format!("The latest sample reports {pct}% utilization."),
            scope: seat_scope(seat),
            evidence: vec![json!({
                "type": "context-usage",
                "nodeId": seat,
                "usedPercentage": pct,
                "available": true,
                "fresh": fresh,
                "observedAt": iso_timestamp(*at),
                "sourceOrder": 0
            })],
            at: *at,
        }
        .finish(),
    ]
}

fn percent(used: u64, limit: u64) -> u32 {
    let raw = used.saturating_mul(100) / limit;
    u32::try_from(raw).unwrap_or(u32::MAX)
}
