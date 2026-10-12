//! Escalate a stable inconclusive screen before the launch timeout.

use std::time::Duration;

use crewmux_core::{Clock, Timestamp};
use crewmux_mux::Capture;
use crewmux_types::{LoginKind, ReadinessCode};

use crate::types::Readiness;

/// How many identical captures count as a stable screen.
const STABLE_HITS: u32 = 3;

/// What one observation of a launching seat decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchDecision {
    /// Keep polling.
    Pending,
    /// Positive composer match.
    Ready,
    /// Gate, hard failure, or a stable unrecognised screen.
    Attention {
        /// Attention code.
        code: ReadinessCode,
        /// Captured screen, kept as evidence.
        evidence: String,
    },
    /// The launch timeout fired first. Tests assert this does not happen for a stable screen.
    TimedOut,
    /// Codex login. The daemon records [`crewmux_core::SeatState::LoginRequired`].
    LoginRequired {
        /// Login kind.
        kind: LoginKind,
        /// Evidence.
        evidence: String,
    },
}

/// Per-seat poll state. The clock is supplied by the caller so tests stay manual.
#[derive(Debug)]
pub struct ReadinessWatch {
    started: Timestamp,
    budget: Duration,
    launch_timeout: Duration,
    last_hash: Option<u64>,
    hits: u32,
}

impl ReadinessWatch {
    /// `budget` is the inconclusive window (default 15s). `launch_timeout` is longer.
    #[must_use]
    pub fn new(now: Timestamp, budget: Duration, launch_timeout: Duration) -> Self {
        Self {
            started: now,
            budget,
            launch_timeout,
            last_hash: None,
            hits: 0,
        }
    }

    /// Fold one classification into the watch.
    #[must_use]
    pub fn observe(
        &mut self,
        now: Timestamp,
        verdict: &Readiness,
        capture: &Capture,
    ) -> WatchDecision {
        match verdict {
            Readiness::Ready => WatchDecision::Ready,
            Readiness::Gate(code) => WatchDecision::Attention {
                code: code.clone(),
                evidence: capture.text.clone(),
            },
            Readiness::HardFailure(message) => WatchDecision::Attention {
                code: ReadinessCode::Other(message.clone()),
                evidence: capture.text.clone(),
            },
            Readiness::Booting | Readiness::Inconclusive => self.inconclusive(now, capture),
            Readiness::Login { kind, evidence } => WatchDecision::LoginRequired {
                kind: *kind,
                evidence: evidence.clone(),
            },
        }
    }

    fn inconclusive(&mut self, now: Timestamp, capture: &Capture) -> WatchDecision {
        let elapsed = now.saturating_duration_since(self.started);
        if elapsed >= self.launch_timeout {
            return WatchDecision::TimedOut;
        }
        if self.last_hash == Some(capture.hash) {
            self.hits = self.hits.saturating_add(1);
        } else {
            self.last_hash = Some(capture.hash);
            self.hits = 1;
        }
        if self.hits >= STABLE_HITS && elapsed >= self.budget {
            WatchDecision::Attention {
                code: ReadinessCode::UnrecognisedScreen,
                evidence: capture.text.clone(),
            }
        } else {
            WatchDecision::Pending
        }
    }
}

/// Default inconclusive budget from PE-002.
#[must_use]
pub const fn default_budget() -> Duration {
    Duration::from_secs(15)
}

/// Launch timeout. Stable screens escalate at [`default_budget`], not here.
#[must_use]
pub const fn default_launch_timeout() -> Duration {
    Duration::from_secs(60)
}

/// Advance `clock` only when the caller asks. Tests use [`crewmux_core::ManualClock`].
pub fn stamp(clock: &impl Clock) -> Timestamp {
    clock.now()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use crewmux_core::{Clock, ManualClock, Timestamp};
    use crewmux_mux::{Capture, CaptureRange, FakeMux, Multiplexer, SessionSpec};
    use crewmux_types::ReadinessCode;

    use super::{ReadinessWatch, WatchDecision};
    use crate::stub::classify;
    use crate::types::Readiness;

    #[tokio::test]
    async fn stable_inconclusive_escalates_before_timeout() {
        let mux = FakeMux::new();
        let spec = SessionSpec::new("static", PathBuf::from("/tmp"));
        let pane = mux.create_session(&spec).await.expect("session");
        let screen = "stub-agent\nstill starting\n";
        mux.set_capture(Capture::new(screen.to_owned(), (0, 1)))
            .await;
        let clock = ManualClock::new(Timestamp::from_millis(0));
        let mut watch = ReadinessWatch::new(
            clock.now(),
            Duration::from_secs(15),
            Duration::from_secs(60),
        );
        let first = mux
            .capture(&pane, CaptureRange::Visible)
            .await
            .expect("capture");
        let verdict = classify(&first.text);
        assert_eq!(verdict, Readiness::Inconclusive);
        assert_eq!(
            watch.observe(clock.now(), &verdict, &first),
            WatchDecision::Pending
        );
        clock.advance(Duration::from_secs(1));
        let second = mux
            .capture(&pane, CaptureRange::Visible)
            .await
            .expect("capture");
        assert_eq!(second.hash, first.hash);
        assert_eq!(
            watch.observe(clock.now(), &classify(&second.text), &second),
            WatchDecision::Pending
        );
        clock.advance(Duration::from_secs(14));
        let third = mux
            .capture(&pane, CaptureRange::Visible)
            .await
            .expect("capture");
        let decision = watch.observe(clock.now(), &classify(&third.text), &third);
        assert_eq!(
            decision,
            WatchDecision::Attention {
                code: ReadinessCode::UnrecognisedScreen,
                evidence: screen.to_owned(),
            }
        );
        assert!(clock.now().millis() < 60_000);
        assert_ne!(decision, WatchDecision::TimedOut);
    }
}
