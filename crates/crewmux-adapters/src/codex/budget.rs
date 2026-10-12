//! Launch timings from the Mac rerun (codex-cli 0.162.0), with margin.

use std::time::Duration;

/// How long the Codex seat waits at each launch stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaunchBudget {
    /// Delay between screen polls.
    pub poll: Duration,
    /// Time allowed for the first classified screen.
    pub first_screen: Duration,
    /// Time allowed to reach the composer after the last answered gate.
    pub ready_after_last_gate: Duration,
    /// Stable inconclusive screen before attention.
    pub inconclusive_stable: Duration,
    /// Whole launch, including gates.
    pub overall: Duration,
    /// How long after the first submit the rollout id may stay unresolved.
    pub thread_id_after_submit: Duration,
}

impl LaunchBudget {
    /// Margins over the observed Mac timings.
    #[must_use]
    pub const fn codex() -> Self {
        Self {
            poll: Duration::from_millis(100),
            first_screen: Duration::from_secs(5),
            ready_after_last_gate: Duration::from_secs(10),
            inconclusive_stable: Duration::from_secs(15),
            overall: Duration::from_secs(45),
            thread_id_after_submit: Duration::from_secs(10),
        }
    }
}
