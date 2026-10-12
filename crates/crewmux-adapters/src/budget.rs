//! Launch timeouts from the Claude Code 2.1.286 Mac timings.

use std::time::Duration;

/// How long a Claude launch may wait. Every field is overridable per launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaunchBudget {
    /// Blank screen. About six times the trusted-folder worst case.
    pub first_screen: Duration,
    /// Composer after the last answered gate. About twice the 6.4s outlier.
    pub ready_after_last_gate: Duration,
    /// Identical inconclusive screen, then attention. Never treated as ready.
    pub inconclusive_stable: Duration,
    /// Hard stop for the whole launch.
    pub overall: Duration,
    /// Gap between captures.
    pub poll: Duration,
}

impl LaunchBudget {
    /// Defaults from the Mac rerun. `poll` is 100ms and `overall` is 45s.
    #[must_use]
    pub const fn claude() -> Self {
        Self {
            first_screen: Duration::from_secs(5),
            ready_after_last_gate: Duration::from_secs(15),
            inconclusive_stable: Duration::from_secs(15),
            overall: Duration::from_secs(45),
            poll: Duration::from_millis(100),
        }
    }

    /// How long submit waits for the new user turn. The Mac sends landed well inside this.
    #[must_use]
    pub const fn submit_wait() -> Duration {
        Duration::from_secs(30)
    }
}

/// Optional millisecond overrides from a `RigSpec`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BudgetOverrides {
    /// Replaces [`LaunchBudget::first_screen`] when set.
    pub first_screen: Option<Duration>,
    /// Replaces [`LaunchBudget::ready_after_last_gate`] when set.
    pub ready_after_last_gate: Option<Duration>,
    /// Replaces [`LaunchBudget::inconclusive_stable`] when set.
    pub inconclusive_stable: Option<Duration>,
    /// Replaces [`LaunchBudget::overall`] when set.
    pub overall: Option<Duration>,
    /// Replaces [`LaunchBudget::poll`] when set.
    pub poll: Option<Duration>,
}

impl LaunchBudget {
    /// [`Self::claude`] with any set override applied.
    #[must_use]
    pub fn with_overrides(overrides: BudgetOverrides) -> Self {
        let mut budget = Self::claude();
        if let Some(value) = overrides.first_screen {
            budget.first_screen = value;
        }
        if let Some(value) = overrides.ready_after_last_gate {
            budget.ready_after_last_gate = value;
        }
        if let Some(value) = overrides.inconclusive_stable {
            budget.inconclusive_stable = value;
        }
        if let Some(value) = overrides.overall {
            budget.overall = value;
        }
        if let Some(value) = overrides.poll {
            budget.poll = value;
        }
        budget
    }
}
