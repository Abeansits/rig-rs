// Derived from openrig packages/daemon/src/domain/restore-orchestrator.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Runtime-truth reconciliation.
//!
//! Missing any of the four evidence preconditions is a no-op with a reason,
//! not an error. The upgrade target is `operator_recovered`, never `ready`.

use crate::outcome::SeatRestoreOutcome;

/// The four visible-evidence preconditions.
#[expect(
    clippy::struct_excessive_bools,
    reason = "the four runtime-truth preconditions are independent flags"
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeEvidence {
    /// The prior restore outcome. Only failed and attention can be upgraded.
    pub prior: SeatRestoreOutcome,
    /// Precondition 1: the tmux session exists.
    pub tmux_alive: bool,
    /// Precondition 2: the binding matches the latest session row.
    pub binding_matches: bool,
    /// Precondition 3: a resume token was used for this launch.
    pub resume_token_used: bool,
    /// Precondition 4: the pane is usable, not a gate and not a shell.
    pub pane_usable: bool,
}

/// No-op or an upgrade. There is no ready outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TruthReconcile {
    /// At least one precondition is missing, or the prior outcome cannot move.
    Noop {
        /// Stable reason code.
        reason: &'static str,
    },
    /// All four preconditions hold.
    OperatorRecovered,
}

/// Reconcile one seat. This never returns an error for missing evidence.
#[must_use]
pub fn reconcile_runtime_truth(evidence: &RuntimeEvidence) -> TruthReconcile {
    if !matches!(
        evidence.prior,
        SeatRestoreOutcome::Failed(_) | SeatRestoreOutcome::AttentionRequired(_)
    ) {
        return TruthReconcile::Noop {
            reason: "outcome_not_upgradable",
        };
    }
    if !evidence.tmux_alive {
        return TruthReconcile::Noop {
            reason: "tmux_session_missing",
        };
    }
    if !evidence.binding_matches {
        return TruthReconcile::Noop {
            reason: "binding_mismatch",
        };
    }
    if !evidence.resume_token_used {
        return TruthReconcile::Noop {
            reason: "resume_token_not_used",
        };
    }
    if !evidence.pane_usable {
        return TruthReconcile::Noop {
            reason: "pane_not_usable",
        };
    }
    TruthReconcile::OperatorRecovered
}

#[cfg(test)]
mod tests {
    use super::{RuntimeEvidence, TruthReconcile, reconcile_runtime_truth};
    use crate::outcome::SeatRestoreOutcome;
    use crewmux_types::ReadinessCode;

    fn full(prior: SeatRestoreOutcome) -> RuntimeEvidence {
        RuntimeEvidence {
            prior,
            tmux_alive: true,
            binding_matches: true,
            resume_token_used: true,
            pane_usable: true,
        }
    }

    #[test]
    fn missing_evidence_is_a_noop_with_a_reason() {
        let mut evidence = full(SeatRestoreOutcome::AttentionRequired(
            ReadinessCode::TrustGate,
        ));
        evidence.tmux_alive = false;
        assert_eq!(
            reconcile_runtime_truth(&evidence),
            TruthReconcile::Noop {
                reason: "tmux_session_missing"
            }
        );
        evidence.tmux_alive = true;
        evidence.pane_usable = false;
        assert_eq!(
            reconcile_runtime_truth(&evidence),
            TruthReconcile::Noop {
                reason: "pane_not_usable"
            }
        );
        evidence.pane_usable = true;
        evidence.resume_token_used = false;
        assert_eq!(
            reconcile_runtime_truth(&evidence),
            TruthReconcile::Noop {
                reason: "resume_token_not_used"
            }
        );
        assert_eq!(
            reconcile_runtime_truth(&full(SeatRestoreOutcome::Resumed)),
            TruthReconcile::Noop {
                reason: "outcome_not_upgradable"
            }
        );
    }

    #[test]
    fn all_four_preconditions_upgrade_to_operator_recovered() {
        let evidence = full(SeatRestoreOutcome::Failed("down".to_owned()));
        assert_eq!(
            reconcile_runtime_truth(&evidence),
            TruthReconcile::OperatorRecovered
        );
    }
}
