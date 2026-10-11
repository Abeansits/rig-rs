//! Shared IDs, enums and wire DTOs; no logic.

mod error;
mod id;
mod status;

pub use error::IdError;
pub use id::{PodId, PodName, RigId, RigName, SeatId, SeatName, SessionId};
pub use status::{LauncherKind, ReadinessCode, RuntimeKind, SeatState};
