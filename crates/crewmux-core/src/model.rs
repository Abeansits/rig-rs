//! Rigs, seats, and seat state.

use std::path::PathBuf;

use crewmux_types::{ReadinessCode, RigId, RigName, RuntimeKind, SeatId, SeatName};
use serde::{Deserialize, Serialize};

/// A rig row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rig {
    /// Identifier.
    pub id: RigId,
    /// Spec name.
    pub name: RigName,
    /// When the rig was planned.
    pub created_at: crate::clock::Timestamp,
}

/// Lifecycle of one seat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeatState {
    /// Parsed from the spec, not yet launched.
    Planned,
    /// The tmux session exists and readiness is still open.
    Launching,
    /// The stub composer matched.
    Ready,
    /// A gate, an invalid cwd, or an unrecognised screen.
    Attention(ReadinessCode),
    /// `rig down` stopped the session.
    Stopped,
    /// The pane process exited.
    Dead,
}

impl SeatState {
    /// Wire token, without the attention code.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Launching => "launching",
            Self::Ready => "ready",
            Self::Attention(_) => "attention",
            Self::Stopped => "stopped",
            Self::Dead => "dead",
        }
    }

    /// Attention code, when the state carries one.
    #[must_use]
    pub const fn code(&self) -> Option<&ReadinessCode> {
        match self {
            Self::Attention(code) => Some(code),
            Self::Planned | Self::Launching | Self::Ready | Self::Stopped | Self::Dead => None,
        }
    }
}

/// A seat row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seat {
    /// Identifier.
    pub id: SeatId,
    /// Owning rig.
    pub rig_id: RigId,
    /// Spec name.
    pub name: SeatName,
    /// Runtime.
    pub runtime: RuntimeKind,
    /// Working directory, when the spec set one.
    pub cwd: Option<PathBuf>,
    /// Model pin, when the spec set one.
    pub model: Option<String>,
    /// Lifecycle.
    pub state: SeatState,
    /// Why a launch failed or a gate fired.
    pub reason: Option<String>,
    /// tmux session name, once launched.
    pub tmux_session: Option<String>,
    /// tmux pane id (`%N`), once launched.
    pub pane_id: Option<String>,
}
