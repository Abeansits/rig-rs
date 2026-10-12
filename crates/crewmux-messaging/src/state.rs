// Derived from openrig packages/daemon/src/domain/queue-repository.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Queue states. [`QueueState::is_terminal`] is the one archiver predicate (TS FS-1).

use serde::{Deserialize, Serialize};

/// `QUEUE_STATES` from the TypeScript repository.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QueueState {
    /// Waiting to be claimed.
    Pending,
    /// Claimed.
    InProgress,
    /// Closed with a closure reason.
    Done,
    /// Parked on a blocker.
    Blocked,
    /// The attempt failed.
    Failed,
    /// The receiver refused.
    Denied,
    /// Withdrawn. Guarded, but not an archiver terminal state.
    Canceled,
    /// Handed to another seat. Archiver-terminal.
    HandedOff,
}

impl QueueState {
    /// `done` and `handed-off` only. Shared by the archiver and the closure guards.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::HandedOff)
    }

    /// `done`, `canceled`, and `handed-off` refuse a new state without `--reopen`.
    #[must_use]
    pub const fn is_guarded(self) -> bool {
        matches!(self, Self::Done | Self::Canceled | Self::HandedOff)
    }

    /// `pending`, `in-progress`, and `blocked`.
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Pending | Self::InProgress | Self::Blocked)
    }

    /// Wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in-progress",
            Self::Done => "done",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
            Self::Denied => "denied",
            Self::Canceled => "canceled",
            Self::HandedOff => "handed-off",
        }
    }

    /// Parse a wire token.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "pending" => Self::Pending,
            "in-progress" => Self::InProgress,
            "done" => Self::Done,
            "blocked" => Self::Blocked,
            "failed" => Self::Failed,
            "denied" => Self::Denied,
            "canceled" => Self::Canceled,
            "handed-off" => Self::HandedOff,
            _ => return None,
        })
    }

    /// Every state, in TypeScript order.
    #[must_use]
    pub const fn all() -> [Self; 8] {
        [
            Self::Pending,
            Self::InProgress,
            Self::Done,
            Self::Blocked,
            Self::Failed,
            Self::Denied,
            Self::Canceled,
            Self::HandedOff,
        ]
    }
}
