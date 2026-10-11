//! Stub screen classifier. A gate wins over `stub> `, and Ready needs that prompt.

use crewmux_types::ReadinessCode;

use crate::types::Readiness;

/// Classify a stub pane.
///
/// Gate lines (`stub gate: <code>`) are attention even when `stub> ` is also present.
/// Anything else that lacks the composer is inconclusive.
#[must_use]
pub fn classify(text: &str) -> Readiness {
    if let Some(code) = gate_code(text) {
        return Readiness::Gate(code);
    }
    if text.contains("stub> ") {
        return Readiness::Ready;
    }
    Readiness::Inconclusive
}

fn gate_code(text: &str) -> Option<ReadinessCode> {
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("stub gate:") else {
            continue;
        };
        let code = rest.trim();
        if code.is_empty() {
            continue;
        }
        return Some(ReadinessCode::parse(code));
    }
    None
}
