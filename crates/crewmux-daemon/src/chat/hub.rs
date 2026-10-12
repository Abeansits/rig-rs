//! In-process wakeups for chat watchers.
//!
//! This is not slice 4's event bus. A committed chat row notifies watchers,
//! and they read the new rows back from SQLite so order and resume stay in the log.

use crewmux_types::RigId;
use tokio::sync::broadcast;

/// Fan-out of "a chat row was committed for this rig".
#[derive(Clone, Debug)]
pub(crate) struct ChatHub {
    tx: broadcast::Sender<RigId>,
}

impl ChatHub {
    pub(crate) fn new() -> Self {
        let (tx, _) = broadcast::channel(64);
        Self { tx }
    }

    /// Wake watchers. No listeners is not an error.
    pub(crate) fn notify(&self, rig: RigId) {
        drop(self.tx.send(rig));
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<RigId> {
        self.tx.subscribe()
    }
}
