//! Daemon input taps. A keystroke defers delivery even with no tmux human client.

use std::time::Instant;

use crewmux_types::SeatId;

/// Records terminal keystrokes that tmux `client_activity` does not see.
pub trait SeatInputTap: Send + Sync {
    /// Remember that `seat` received a key at `at`.
    fn record_keystroke(&self, seat: &SeatId, at: Instant);
}
