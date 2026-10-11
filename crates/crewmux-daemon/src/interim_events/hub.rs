//! TEMPORARY in-process bus for [`super::serve`].
//!
//! Slice 4 replaces this hub along with the route. It maps [`crewmux_core::DomainEvent`]
//! onto the client [`crewmux_api::StreamEvent`] and drops the event when nobody is listening.

use tokio::sync::broadcast;

use crewmux_api::StreamEvent;
use crewmux_core::DomainEvent;

/// How many events a slow subscriber may miss before `Lagged`.
const CAPACITY: usize = 64;

/// In-process bus. `GET /api/events` subscribes; publishers never wait on a client.
#[derive(Clone, Debug)]
pub struct EventHub {
    tx: broadcast::Sender<StreamEvent>,
}

impl EventHub {
    /// Empty bus. Publishing with no subscribers drops the event.
    #[must_use]
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(CAPACITY);
        Self { tx }
    }

    /// Map a domain event onto the wire and fan it out.
    pub fn publish(&self, event: &DomainEvent) {
        let wire = match event {
            DomainEvent::SeatStateChanged { seat, state } => StreamEvent::SeatStateChanged {
                seat: seat.clone(),
                state: state.clone(),
            },
            DomainEvent::SeatDied { seat } => StreamEvent::SeatDied { seat: seat.clone() },
            DomainEvent::Activity { seat, summary } => StreamEvent::Activity {
                seat: seat.clone(),
                summary: summary.clone(),
            },
        };
        // No listeners is normal before the TUI connects.
        if self.tx.send(wire).is_err() {
            tracing::debug!("event dropped; no subscribers");
        }
    }

    /// Subscribe. The receiver misses events sent before this call.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<StreamEvent> {
        self.tx.subscribe()
    }
}

impl Default for EventHub {
    fn default() -> Self {
        Self::new()
    }
}
