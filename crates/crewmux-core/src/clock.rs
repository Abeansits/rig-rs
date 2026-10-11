//! Time as milliseconds, so lifecycle tests can use a manual clock.

use std::cell::Cell;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Milliseconds since the Unix epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(i64);

impl Timestamp {
    /// A timestamp from a millisecond count.
    #[must_use]
    pub const fn from_millis(millis: i64) -> Self {
        Self(millis)
    }

    /// Milliseconds since the epoch.
    #[must_use]
    pub const fn millis(self) -> i64 {
        self.0
    }

    /// Add a duration, saturating at [`i64::MAX`].
    #[must_use]
    pub fn saturating_add(self, duration: Duration) -> Self {
        let millis = i64::try_from(duration.as_millis()).unwrap_or(i64::MAX);
        Self(self.0.saturating_add(millis))
    }

    /// Time from `earlier` to `self`. Zero when `self` is earlier.
    #[must_use]
    pub fn saturating_duration_since(self, earlier: Self) -> Duration {
        let delta = self.0.saturating_sub(earlier.0);
        Duration::from_millis(u64::try_from(delta).unwrap_or(0))
    }
}

/// Something that can tell the domain what time it is.
pub trait Clock {
    /// Current time.
    fn now(&self) -> Timestamp;
}

/// Wall clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
            });
        Timestamp(millis)
    }
}

/// A clock tests move by hand. Not shared across threads.
#[derive(Debug)]
pub struct ManualClock {
    now: Cell<Timestamp>,
}

impl ManualClock {
    /// Start at `now`.
    #[must_use]
    pub const fn new(now: Timestamp) -> Self {
        Self {
            now: Cell::new(now),
        }
    }

    /// Move the clock forward.
    pub fn advance(&self, duration: Duration) {
        self.now.set(self.now.get().saturating_add(duration));
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        self.now.get()
    }
}
