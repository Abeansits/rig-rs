//! Facts the engine may see about rigs and seats. The daemon fills this in.

use std::collections::{BTreeMap, BTreeSet};

use crate::role::SeatCandidate;

/// Inventory used by owner resolution, advisories, and the exception dial.
#[derive(Clone, Debug, Default)]
pub struct World {
    /// Rig names `rig ps` would list.
    pub rigs: BTreeSet<String>,
    /// Canonical `member@rig` coordinates that exist, at any lifecycle.
    pub members: BTreeSet<String>,
    /// Canonical session to agent runtime. Present means the lookup exists.
    pub runtimes: BTreeMap<String, String>,
    /// Name of the instance's bound rig, when the call is bound.
    pub bound_rig: Option<String>,
    /// Seats of the bound rig. `None` means the rig name no longer resolves.
    pub bound_seats: Option<Vec<SeatCandidate>>,
    /// Host dial. `orchestrator`, `human_only`, or absent.
    pub host_default: Option<String>,
}

impl World {
    /// Runtime of `session`, when the lookup is installed.
    #[must_use]
    pub fn runtime_of(&self, session: &str) -> Option<String> {
        self.runtimes.get(session).cloned()
    }
}
