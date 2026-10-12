//! Domain events appended in causal order.

use crewmux_types::{QItemId, RigId, RigName, SeatId, SessionId};
use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;

/// Monotonic position in the event log.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Seq(pub u64);

/// Something that happened to a rig or seat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
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
    /// A queue row was created or changed state.
    QueueChanged {
        /// Item.
        qitem: QItemId,
        /// State after the change.
        state: String,
        /// When.
        at: Timestamp,
    },
    /// A direct send recorded a receipt.
    DeliveryRecorded {
        /// Destination seat session.
        seat: String,
        /// `delivered`, `deferred`, `escalated`, `contaminated`, or `failed`.
        outcome: String,
        /// When.
        at: Timestamp,
    },
    /// A rig restore finished. Published on the existing `/api/events` hub.
    RestoreCompleted {
        /// Rig.
        rig: RigId,
        /// Rollup token (`fully_restored`, `partially_restored`, `failed`).
        outcome: String,
        /// When.
        at: Timestamp,
    },
    /// Post-paste contamination, or another attention signal.
    AttentionRaised {
        /// Seat session.
        seat: String,
        /// Why.
        reason: String,
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
            Self::QueueChanged { .. } => "queue_changed",
            Self::DeliveryRecorded { .. } => "delivery_recorded",
            Self::RestoreCompleted { .. } => "restore_completed",
            Self::AttentionRaised { .. } => "attention_raised",
        }
    }

    /// When the event happened.
    #[must_use]
    pub const fn at(&self) -> Timestamp {
        match self {
            Self::RigCreated { at, .. }
            | Self::SeatLaunched { at, .. }
            | Self::SeatReady { at, .. }
            | Self::SeatStopped { at, .. }
            | Self::SeatDied { at, .. }
            | Self::QueueChanged { at, .. }
            | Self::DeliveryRecorded { at, .. }
            | Self::RestoreCompleted { at, .. }
            | Self::AttentionRaised { at, .. } => *at,
        }
    }
}
