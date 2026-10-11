//! Read models the TUI hydrates. Slice 1 serves `ListSeats` and `AttachInfo`;
//! until that lands, the fake API server in `crewmux-tui` test support serves these
//! shapes. Slice 1 must keep the field names.

use crewmux_types::{
    LauncherKind, PodName, RigId, RigName, RuntimeKind, SeatId, SeatName, SeatState, SessionId,
};
use serde::{Deserialize, Serialize};

/// One seat as the explorer shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatView {
    /// Stable id.
    pub id: SeatId,
    /// Display name.
    pub name: SeatName,
    /// Owning rig.
    pub rig_id: RigId,
    /// Rig display name.
    pub rig_name: RigName,
    /// Pod display name.
    pub pod: PodName,
    /// Lifecycle, including `Attention(LoginRequired)`.
    pub state: SeatState,
    /// Runtime.
    pub runtime: RuntimeKind,
    /// tmux session, when the seat has a pane.
    pub session: Option<SessionId>,
    /// Spec name for the seat-detail cross-link. Absent when the daemon has none.
    pub spec: Option<String>,
}

/// `GET /api/seats`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatList {
    /// Every seat the daemon currently knows.
    pub seats: Vec<SeatView>,
}

/// `GET /api/seats/{id}/attach`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachInfo {
    /// Seat the command attaches.
    pub seat: SeatId,
    /// tmux socket name passed to `-L` (the product default is `rig-rs`).
    pub socket: String,
    /// Exact session name. The client passes it as `-t =<session>`.
    pub session: SessionId,
}

/// One health finding. Empty or absent means the daemon has not exposed health yet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthItem {
    /// Finding id.
    pub id: String,
    /// `info`, `warning`, or `critical`.
    pub severity: String,
    /// One-line summary.
    pub summary: String,
}

/// Health block on a snapshot. `None` on the parent means the read is absent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthSummary {
    /// Findings. An empty vec is a successful empty read, not an outage.
    pub records: Vec<HealthItem>,
}

/// One recent-activity line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityItem {
    /// Seat the line is about.
    pub seat: SeatId,
    /// One-line summary.
    pub summary: String,
}

/// Hydrate body. Built from list/attach/health/activity reads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Seats grouped by the client into rigs and pods.
    pub seats: Vec<SeatView>,
    /// `None` when this daemon does not expose health yet.
    pub health: Option<HealthSummary>,
    /// Newest first.
    pub activity: Vec<ActivityItem>,
}

impl Snapshot {
    /// An instance with no rigs.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            seats: Vec::new(),
            health: None,
            activity: Vec::new(),
        }
    }
}

/// Client-side live event on `GET /api/events` (ADR-0006).
///
/// The TUI decodes this enum and does not link the daemon. There is no slice 4
/// spec in the design yet, so the temporary daemon fan-out emits these three
/// variants. Slice 4 owns the route and the transcript writer: it should keep
/// the `type` tag and these names, and may add variants. Unknown tags are
/// ignored by the client, so a newer server does not break this TUI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// Seat lifecycle changed. Applied in place, without a full re-hydrate.
    SeatStateChanged {
        /// Seat.
        seat: SeatId,
        /// New state.
        state: SeatState,
    },
    /// Seat pane died.
    SeatDied {
        /// Seat.
        seat: SeatId,
    },
    /// Activity line.
    Activity {
        /// Seat.
        seat: SeatId,
        /// Summary.
        summary: String,
    },
}

/// Which launcher the operator configured, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LauncherConfig {
    /// Herdr or Ghostty.
    pub kind: LauncherKind,
}
