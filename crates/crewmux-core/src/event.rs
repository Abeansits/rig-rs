//! Domain events appended in causal order.

use crewmux_types::{RigId, RigName, SeatId, SessionId};
use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;

/// Monotonic position in the event log.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Seq(pub u64);

/// Something that happened to a rig or seat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DomainEvent {
    /// `plan_up` accepted a spec.
    RigCreated {
        /// New rig.
        rig: RigId,
        /// Spec name.
        name: RigName,
        /// When.
        at: Timestamp,
    },
    /// A tmux session was created for a seat.
    SeatLaunched {
        /// Seat.
        seat: SeatId,
        /// Session row.
        session: SessionId,
        /// When.
        at: Timestamp,
    },
    /// The composer matched.
    SeatReady {
        /// Seat.
        seat: SeatId,
        /// When.
        at: Timestamp,
    },
    /// `rig down` killed the session.
    SeatStopped {
        /// Seat.
        seat: SeatId,
        /// When.
        at: Timestamp,
    },
    /// The pane process exited.
    SeatDied {
        /// Seat.
        seat: SeatId,
        /// When.
        at: Timestamp,
    },
}

impl DomainEvent {
    /// Wire kind, used by `rig up --json`.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::RigCreated { .. } => "rig_created",
            Self::SeatLaunched { .. } => "seat_launched",
            Self::SeatReady { .. } => "seat_ready",
            Self::SeatStopped { .. } => "seat_stopped",
            Self::SeatDied { .. } => "seat_died",
        }
    }
}
