//! Dialog waits. A live launch pauses; tests advance a manual clock.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;

/// How long [`super::DialogDriver`] waits between captures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DialogPace {
    /// The dialog must be unchanged for this long before the first key.
    pub stable: Duration,
    /// Pause after Up or Down before the next capture.
    pub nav_settle: Duration,
    /// Gap between the two captures that authorise Enter.
    pub confirm_gap: Duration,
}

impl DialogPace {
    /// Mac 2.1.287 timings. The trust dialog redraws about 50 ms after Down.
    #[must_use]
    pub const fn live() -> Self {
        Self {
            stable: Duration::from_secs(1),
            nav_settle: Duration::from_millis(400),
            confirm_gap: Duration::from_millis(400),
        }
    }

    /// No wall wait. Two captures must still agree before Enter.
    #[must_use]
    pub const fn immediate() -> Self {
        Self {
            stable: Duration::ZERO,
            nav_settle: Duration::ZERO,
            confirm_gap: Duration::ZERO,
        }
    }
}

/// Time source for the driver. Tests share one clock with the pane double.
#[async_trait]
pub trait DialogClock: Send + Sync {
    /// Time since this clock was created.
    fn elapsed(&self) -> Duration;

    /// Advance by `duration`. A wall clock sleeps; a manual clock does not.
    async fn wait(&self, duration: Duration);
}

/// `tokio::time::sleep`, skipped when `duration` is zero.
pub struct WallClock {
    started: Instant,
}

impl WallClock {
    /// Start at the current instant.
    #[must_use]
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

impl Default for WallClock {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DialogClock for WallClock {
    fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    async fn wait(&self, duration: Duration) {
        if !duration.is_zero() {
            tokio::time::sleep(duration).await;
        }
    }
}

/// Elapsed time that moves only when [`DialogClock::wait`] is called.
pub struct ManualClock {
    elapsed_ms: AtomicU64,
}

impl ManualClock {
    /// Elapsed time starts at zero.
    #[must_use]
    pub fn new() -> Self {
        Self {
            elapsed_ms: AtomicU64::new(0),
        }
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DialogClock for ManualClock {
    fn elapsed(&self) -> Duration {
        Duration::from_millis(self.elapsed_ms.load(Ordering::Relaxed))
    }

    async fn wait(&self, duration: Duration) {
        let add = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        self.elapsed_ms.fetch_add(add, Ordering::Relaxed);
    }
}
