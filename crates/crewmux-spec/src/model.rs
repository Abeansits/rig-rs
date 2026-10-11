// Derived from openrig packages/daemon/src/domain/rigspec-schema.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! The slice 1 `RigSpec`: a name and a list of seats.
//!
//! Unknown fields are rejected. Only `name`, `runtime`, `cwd`, and `model`
//! are accepted on a seat. Runtimes outside [`RuntimeKind`] (including `pi`)
//! fail at parse time.

use std::path::PathBuf;

use crewmux_types::{RigName, RuntimeKind, SeatName};
use serde::Deserialize;

/// A rig described by a YAML file.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RigSpec {
    /// Rig name.
    pub name: RigName,
    /// Seats to launch. At least one.
    pub seats: Vec<SeatSpec>,
}

/// One seat in a [`RigSpec`].
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeatSpec {
    /// Seat name, unique within the rig.
    pub name: SeatName,
    /// Runtime. `pi` and any other unknown token are a [`crate::SpecError`].
    pub runtime: RuntimeKind,
    /// Working directory. Absent means the daemon's current directory at launch.
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    /// Optional model pin.
    #[serde(default)]
    pub model: Option<String>,
}
