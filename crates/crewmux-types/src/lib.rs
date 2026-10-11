//! Shared IDs, enums, wire DTOs, and the tmux version parser.
//!
//! The version parser lives here so slice 1b can reuse it without this slice
//! linking the tmux control modules.

mod doctor;
mod tmux_version;

pub use doctor::{CheckStatus, DoctorCheck};
pub use tmux_version::{Version, parse_version};
