//! Pure domain events. Slice 1 owns the lifecycle set (`RigCreated`, `SeatLaunched`, …).
//! The variants here feed the temporary `GET /api/events` fan-out. Slice 4's
//! envelope wraps them; this enum is not the wire format.

use crewmux_types::{SeatId, SeatState};

/// A fact the daemon can publish. The HTTP stream maps these onto the API `StreamEvent`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DomainEvent {
    /// A seat moved to a new lifecycle state.
    SeatStateChanged {
        /// Seat that changed.
        seat: SeatId,
        /// State after the change.
        state: SeatState,
    },
    /// The seat's pane died.
    SeatDied {
        /// Seat that died.
        seat: SeatId,
    },
    /// A line of recent activity for a seat.
    Activity {
        /// Seat the line belongs to.
        seat: SeatId,
        /// One-line summary. Nothing is injected into the seat's prompt (ADR-0009).
        summary: String,
    },
}
