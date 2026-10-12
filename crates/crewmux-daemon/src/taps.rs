//! Keystroke taps and draft ages for the guard.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crewmux_mux::SeatInputTap;
use crewmux_types::SeatId;
use tokio::sync::Mutex;

/// Last keystroke per seat name or seat id.
#[derive(Debug, Default)]
pub(crate) struct KeyTaps {
    inner: Mutex<HashMap<String, Instant>>,
}

impl KeyTaps {
    pub(crate) fn record_name(&self, seat: &str, at: Instant) {
        if let Ok(mut guard) = self.inner.try_lock() {
            guard.insert(seat.to_owned(), at);
        }
    }

    pub(crate) fn last(&self, seat: &str) -> Option<Instant> {
        self.inner
            .try_lock()
            .ok()
            .and_then(|guard| guard.get(seat).copied())
    }
}

impl SeatInputTap for KeyTaps {
    fn record_keystroke(&self, seat: &SeatId, at: Instant) {
        self.record_name(&seat.to_string(), at);
    }
}

/// When a stub draft was first seen, so rule 4 can escalate.
#[derive(Debug, Default)]
pub(crate) struct DraftAges {
    inner: Mutex<HashMap<String, Instant>>,
}

impl DraftAges {
    pub(crate) async fn observe(
        &self,
        seat: &str,
        drafting: bool,
        now: Instant,
    ) -> Option<Duration> {
        let mut guard = self.inner.lock().await;
        if !drafting {
            guard.remove(seat);
            return None;
        }
        let start = *guard.entry(seat.to_owned()).or_insert(now);
        Some(now.saturating_duration_since(start))
    }
}
