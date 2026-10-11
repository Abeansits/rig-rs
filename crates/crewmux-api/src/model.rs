//! Read models the TUI hydrates. Slice 1 serves `ListSeats` and `AttachInfo`;
//! until that lands, the fake API server in `crewmux-tui` test support serves these
//! shapes. Slice 1 must keep the field names.

use crewmux_core::SeatState;
use crewmux_types::{LauncherKind, PodName, RigName, RuntimeKind, SeatKey, TmuxSession};
use serde::{Deserialize, Serialize};

/// One seat as the explorer shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatView {
    /// Stable id.
    pub id: SeatKey,
    /// Display name.
    pub name: SeatKey,
    /// Owning rig.
    pub rig_id: RigName,
    /// Rig display name.
    pub rig_name: RigName,
    /// Pod display name.
    pub pod: PodName,
    /// Lifecycle, including `Attention(LoginRequired)`.
    pub state: SeatState,
    /// Runtime.
    pub runtime: RuntimeKind,
    /// tmux session, when the seat has a pane.
    pub session: Option<TmuxSession>,
    /// Spec name for the seat-detail cross-link. Absent when the daemon has none.
    pub spec: Option<String>,
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
    pub seat: SeatKey,
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

/// Envelope on `GET /api/events`.
///
/// Slice 4 owns the canonical stream: this id, a `type`, and a payload. Domain
/// tags (`seat_state_changed`, `seat_died`, `activity`) are the `type`. `gap`
/// has no payload and means the client missed events. The TUI decodes this
/// struct and does not link the daemon. Unknown `type` values fail to decode
/// and are skipped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamEvent {
    /// Resume cursor. The client sends it back as `Last-Event-ID`.
    pub id: u64,
    /// Kind. Wire name is `type`.
    #[serde(rename = "type")]
    pub kind: StreamKind,
    /// Domain fields. Absent for [`StreamKind::Gap`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<StreamPayload>,
}

/// `type` on a [`StreamEvent`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamKind {
    /// Seat lifecycle changed. Applied in place.
    SeatStateChanged,
    /// Seat pane died.
    SeatDied,
    /// Activity line.
    Activity,
    /// The server could not replay missed events. The client re-hydrates.
    Gap,
}

/// Domain fields inside a [`StreamEvent`]. Which fields are set depends on [`StreamKind`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamPayload {
    /// Seat the event is about.
    pub seat: SeatKey,
    /// New lifecycle state. Set for `seat_state_changed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<SeatState>,
    /// One-line summary. Set for `activity`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// Which launcher the operator configured, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LauncherConfig {
    /// Herdr or Ghostty.
    pub kind: LauncherKind,
}
