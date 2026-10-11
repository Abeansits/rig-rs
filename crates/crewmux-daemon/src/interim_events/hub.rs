//! TEMPORARY in-process bus for [`super::serve`].
//!
//! Slice 4 replaces this hub along with the route. It maps [`crewmux_core::DomainEvent`]
//! onto the client envelope and drops the event when nobody is listening. Ids are
//! assigned here. There is no backlog.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::broadcast;

use crewmux_api::{StreamEvent, StreamKind, StreamPayload};
use crewmux_core::DomainEvent;

/// How many events a slow subscriber may miss before `Lagged`.
const CAPACITY: usize = 64;

/// In-process bus. `GET /api/events` subscribes; publishers never wait on a client.
#[derive(Clone, Debug)]
pub struct EventHub {
    tx: broadcast::Sender<StreamEvent>,
    next_id: Arc<AtomicU64>,
}

impl EventHub {
    /// Empty bus. Publishing with no subscribers drops the event.
    #[must_use]
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(CAPACITY);
        Self {
            tx,
            next_id: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Map a domain event onto the wire envelope and fan it out.
    pub fn publish(&self, event: &DomainEvent) {
        let wire = self.envelope(event);
        // No listeners is normal before the TUI connects.
        if self.tx.send(wire).is_err() {
            tracing::debug!("event dropped; no subscribers");
        }
    }

    /// One `gap` envelope. Used when a client resumes and this hub has no backlog.
    #[must_use]
    pub fn gap(&self) -> StreamEvent {
        StreamEvent {
            id: self.alloc_id(),
            kind: StreamKind::Gap,
            payload: None,
        }
    }

    /// Subscribe. The receiver misses events sent before this call.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<StreamEvent> {
        self.tx.subscribe()
    }

    fn envelope(&self, event: &DomainEvent) -> StreamEvent {
        let (kind, payload) = match event {
            DomainEvent::SeatStateChanged { seat, state } => (
                StreamKind::SeatStateChanged,
                StreamPayload {
                    seat: seat.clone(),
                    state: Some(state.clone()),
                    summary: None,
                },
            ),
            DomainEvent::SeatDied { seat } => (
                StreamKind::SeatDied,
                StreamPayload {
                    seat: seat.clone(),
                    state: None,
                    summary: None,
                },
            ),
            DomainEvent::Activity { seat, summary } => (
                StreamKind::Activity,
                StreamPayload {
                    seat: seat.clone(),
                    state: None,
                    summary: Some(summary.clone()),
                },
            ),
        };
        StreamEvent {
            id: self.alloc_id(),
            kind,
            payload: Some(payload),
        }
    }

    fn alloc_id(&self) -> u64 {
        self.next_id
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1)
    }
}

impl Default for EventHub {
    fn default() -> Self {
        Self::new()
    }
}
