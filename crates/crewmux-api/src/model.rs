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

/// Which launcher the operator configured, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LauncherConfig {
    /// Herdr or Ghostty.
    pub kind: LauncherKind,
}
