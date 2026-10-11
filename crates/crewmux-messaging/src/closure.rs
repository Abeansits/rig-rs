// Derived from openrig packages/daemon/src/domain/hot-potato-enforcer.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Hot-potato closure rules. Messages match the TypeScript validator.

use std::time::Duration;

use crewmux_core::Timestamp;
use serde::{Deserialize, Serialize};

use crate::error::Error;

/// The seven closure reasons, including `superseded`.
pub const CLOSURE_REASONS: &[&str] = &[
    "handed_off_to",
    "blocked_on",
    "denied",
    "canceled",
    "no-follow-on",
    "escalation",
    "superseded",
];

/// A recorded closure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Closure {
    /// One of [`CLOSURE_REASONS`].
    pub reason: String,
    /// Required for `handed_off_to`, `blocked_on`, `escalation`, and `superseded`.
    pub target: Option<String>,
    /// SLA instant, when the claim tier computed one.
    pub required_at: Option<Timestamp>,
}

/// Validate a transition into `state`.
///
/// # Errors
///
/// Returns the TypeScript `missing_closure_reason`, `invalid_closure_reason`,
/// or `missing_closure_target` sentence.
pub fn validate_closure(
    state: &str,
    reason: Option<&str>,
    target: Option<&str>,
) -> Result<Option<Closure>, Error> {
    if state != "done" {
        return Ok(reason.map(|reason| Closure {
            reason: reason.to_owned(),
            target: target.map(str::to_owned),
            required_at: None,
        }));
    }
    let Some(reason) = reason.filter(|reason| !reason.is_empty()) else {
        return Err(missing_reason());
    };
    if !CLOSURE_REASONS.contains(&reason) {
        return Err(Error::Rejected {
            code: "invalid_closure_reason",
            message: format!(
                "closure_reason={reason} is not valid; valid values: {}",
                CLOSURE_REASONS.join(", ")
            ),
        });
    }
    let needs_target = matches!(reason, "handed_off_to" | "blocked_on" | "escalation");
    if needs_target && target.unwrap_or("").is_empty() {
        return Err(Error::Rejected {
            code: "missing_closure_target",
            message: format!("closure_reason={reason} requires closure_target"),
        });
    }
    if reason == "superseded" && target.unwrap_or("").is_empty() {
        return Err(Error::Rejected {
            code: "missing_closure_target",
            message: "closure_reason=superseded requires closure_target (the successor qitem that replaced this row).".to_owned(),
        });
    }
    Ok(Some(Closure {
        reason: reason.to_owned(),
        target: target.map(str::to_owned),
        required_at: None,
    }))
}

fn missing_reason() -> Error {
    Error::Rejected {
        code: "missing_closure_reason",
        message: format!(
            "state=done requires closure_reason; valid values: {}",
            CLOSURE_REASONS.join(", ")
        ),
    }
}

/// Tier SLA from the claim instant. Unknown tiers have no deadline.
#[must_use]
pub fn closure_required_at(claimed: Timestamp, tier: Option<&str>) -> Option<Timestamp> {
    let seconds = match tier? {
        "fast" => 30 * 60,
        "routine" => 4 * 60 * 60,
        "deep" => 24 * 60 * 60,
        "critical" => 15 * 60,
        _ => return None,
    };
    Some(claimed.saturating_add(Duration::from_secs(seconds)))
}
