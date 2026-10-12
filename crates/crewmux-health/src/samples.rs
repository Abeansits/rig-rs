// Derived from openrig packages/daemon/src/domain/health-detectors.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Context samples the daemon read from `context_usage`.

use crewmux_core::Timestamp;
use crewmux_messaging::SessionRef;

use crate::coverage::Coverage;
use crate::observation::HealthObservation;

/// One stored context sample. A missing limit is not zero percent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeatSample {
    /// Seat session.
    pub seat: SessionRef,
    /// Tokens in use. `None` is unknown, not zero.
    pub used_tokens: Option<u64>,
    /// Context window. `None` or zero is unavailable.
    pub limit: Option<u64>,
    /// When the sample was taken.
    pub at: Timestamp,
}

/// Observations for samples that have a positive limit. Every sample is evaluated.
///
/// Samples past `cap` are omitted and are not a healthy verdict.
#[must_use]
pub fn context_observations(
    samples: &[SeatSample],
    cap: u32,
) -> (Vec<HealthObservation>, Coverage) {
    let total = u32::try_from(samples.len()).unwrap_or(u32::MAX);
    let keep = usize::try_from(cap).unwrap_or(usize::MAX);
    let mut observations = Vec::new();
    for sample in samples.iter().take(keep) {
        let (Some(used), Some(limit)) = (sample.used_tokens, sample.limit) else {
            continue;
        };
        if limit == 0 {
            continue;
        }
        observations.push(HealthObservation::ContextPressure {
            seat: sample.seat.clone(),
            used_tokens: used,
            limit,
            at: sample.at,
        });
    }
    let evaluated = total.min(cap);
    let omitted = total.saturating_sub(evaluated);
    (
        observations,
        Coverage {
            evaluated,
            omitted,
            limit_hit: omitted > 0,
        },
    )
}
