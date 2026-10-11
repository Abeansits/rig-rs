//! Shared IDs, enums, wire DTOs, and the tmux version parser.
//!
//! [`tmux_version`] is the canonical parser. Slice 1b re-exports it instead of
//! keeping a second copy next to the tmux control modules.

mod doctor;
pub mod tmux_version;

pub use doctor::{CheckStatus, DoctorCheck};
pub use tmux_version::{Version, parse_version};
