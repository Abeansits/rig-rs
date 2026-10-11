//! Shared IDs, enums, wire DTOs, and the tmux version parser.
//!
//! [`tmux_version`] is the canonical parser. Slice 1b re-exports it instead of
//! keeping a second copy next to the tmux control modules.
//!
//! Seat identity stays in [`id`] and [`status`] so other slices can add their
//! own modules beside these.

mod doctor;
mod error;
mod id;
mod status;
pub mod tmux_version;

pub use doctor::{CheckStatus, DoctorCheck};
pub use error::IdError;
pub use id::{PodId, PodName, RigId, RigName, SeatId, SeatName, SessionId};
pub use status::{LauncherKind, ReadinessCode, RuntimeKind, SeatState};
pub use tmux_version::{Version, parse_version};
