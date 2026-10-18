// Derived from openrig packages/daemon/src/domain/watchdog-due.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
// Derived from openrig packages/daemon/src/domain/policies/workflow-keepalive.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! When a watchdog job is due, and what a due keepalive or owner-wake sends.
//!
//! The daemon claims the occurrence and pastes. This module does not touch
//! SQLite, a process, or a pane.

mod due;
mod owner;
mod policy;
mod spec;
mod text;

pub use due::{Schedule, is_due, next_due_at};
pub use owner::{OwnerWake, owner_wakes};
pub use policy::{KeepaliveJob, WatchAction, WatchFail, evaluate_keepalive, is_quiet};
pub use spec::{ArmedSpec, parse_armed};
pub use text::{keepalive_message, occurrence_id, stuck_message};
