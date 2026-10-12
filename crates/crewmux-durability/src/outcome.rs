//! Per-seat and rig restore outcomes.
//!
//! `ready` is not a variant. A post-reconciliation success is
//! [`SeatRestoreOutcome::OperatorRecovered`].

use crewmux_types::ReadinessCode;
use serde::{Deserialize, Serialize};

/// What restore concluded for one seat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeatRestoreOutcome {
    /// `launch(resume)` replayed the existing transcript.
    Resumed,
    /// No resume token. The seat started empty.
    Fresh,
    /// A gate or other attention code. Not ready.
    AttentionRequired(ReadinessCode),
    /// The relaunch failed.
    Failed(String),
    /// Runtime-truth reconciliation upgraded a failure. Never `ready`.
    OperatorRecovered,
}

impl SeatRestoreOutcome {
    /// Wire token. Attention and failure carry their payload separately.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Resumed => "resumed",
            Self::Fresh => "fresh",
            Self::AttentionRequired(_) => "attention_required",
            Self::Failed(_) => "failed",
            Self::OperatorRecovered => "operator_recovered",
        }
    }
}

/// Fleet or rig rollup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RigRestoreOutcome {
    /// Every seat is resumed, fresh, or operator-recovered.
    FullyRestored,
    /// At least one seat needs attention or failed, and not every seat failed.
    PartiallyRestored,
    /// Every seat failed, or the rig had no seats.
    Failed,
    /// Crash-cart did not start this rig.
    NotAttempted,
}

impl RigRestoreOutcome {
    /// Wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FullyRestored => "fully_restored",
            Self::PartiallyRestored => "partially_restored",
            Self::Failed => "failed",
            Self::NotAttempted => "not_attempted",
        }
    }
}

/// Roll seats up. Fresh counts as restored (slice 8 AC3).
///
/// The TypeScript `rollupRestoreRigResult` treats `fresh` as partial. This slice's
/// acceptance says a resumed seat plus a fresh seat is [`RigRestoreOutcome::FullyRestored`].
#[must_use]
pub fn rollup(seats: &[SeatRestoreOutcome]) -> RigRestoreOutcome {
    if seats.is_empty()
        || seats
            .iter()
            .all(|seat| matches!(seat, SeatRestoreOutcome::Failed(_)))
    {
        return RigRestoreOutcome::Failed;
    }
    let blocked = seats.iter().any(|seat| {
        matches!(
            seat,
            SeatRestoreOutcome::AttentionRequired(_) | SeatRestoreOutcome::Failed(_)
        )
    });
    if blocked {
        RigRestoreOutcome::PartiallyRestored
    } else {
        RigRestoreOutcome::FullyRestored
    }
}

#[cfg(test)]
mod tests {
    use super::{RigRestoreOutcome, SeatRestoreOutcome, rollup};
    use crewmux_types::ReadinessCode;

    #[test]
    fn fresh_with_resumed_is_fully_restored_and_never_ready() {
        let seats = [
            SeatRestoreOutcome::Resumed,
            SeatRestoreOutcome::Fresh,
            SeatRestoreOutcome::OperatorRecovered,
        ];
        assert_eq!(rollup(&seats), RigRestoreOutcome::FullyRestored);
        for seat in &seats {
            assert_ne!(seat.as_str(), "ready");
        }
    }

    #[test]
    fn attention_rolls_up_partial_and_all_failed_is_failed() {
        let gate = SeatRestoreOutcome::AttentionRequired(ReadinessCode::TrustGate);
        assert_eq!(
            rollup(&[SeatRestoreOutcome::Resumed, gate.clone()]),
            RigRestoreOutcome::PartiallyRestored
        );
        assert_eq!(gate.as_str(), "attention_required");
        assert_eq!(
            rollup(&[SeatRestoreOutcome::Failed("x".to_owned())]),
            RigRestoreOutcome::Failed
        );
    }
}
