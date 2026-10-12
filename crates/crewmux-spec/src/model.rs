// Derived from openrig packages/daemon/src/domain/rigspec-schema.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! The `RigSpec`: a name and a list of seats.
//!
//! Unknown fields are rejected. A seat accepts `name`, `runtime`, `cwd`,
//! `model`, `auto_trust`, and `launch`. Runtimes outside [`RuntimeKind`]
//! (including `pi`) fail at parse time.

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
    /// Answer the Claude trust dialog. Absent means false.
    #[serde(default)]
    pub auto_trust: bool,
    /// Millisecond overrides for the Claude launch budget.
    #[serde(default)]
    pub launch: Option<LaunchBudgetSpec>,
}

/// Optional Claude launch timeouts, in milliseconds.
///
/// Absent fields keep the Claude adapter defaults. The daemon converts these
/// numbers; this crate does not depend on the adapter.
#[derive(Clone, Debug, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct LaunchBudgetSpec {
    /// Blank-screen budget, in milliseconds.
    pub first_screen: Option<u64>,
    /// Composer budget after the last answered dialog, in milliseconds.
    pub ready_after_last_gate: Option<u64>,
    /// Stable inconclusive screen budget, in milliseconds.
    pub inconclusive_stable: Option<u64>,
    /// Hard stop for the whole launch, in milliseconds.
    pub overall: Option<u64>,
    /// Gap between captures, in milliseconds.
    pub poll: Option<u64>,
}
