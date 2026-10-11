//! Shared IDs, enums and wire DTOs.
//!
//! [`Key`] is the multiplexer key enum from slice 1b. The rest of this crate
//! is the slice 1 seat vocabulary: ids, runtimes, and readiness codes.

mod error;
mod id;
mod readiness;
mod runtime;

pub use error::TypeError;
pub use id::{RigId, RigName, SeatId, SeatName, SessionId};
pub use readiness::ReadinessCode;
pub use runtime::{ForkSource, LaunchPosture, RuntimeKind};

/// A key delivered with `send-keys`.
///
/// [`Key::Literal`] is a single line. Multi-line text goes through paste.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    /// Carriage return.
    Enter,
    /// Escape.
    Escape,
    /// Control-C.
    CtrlC,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Tab.
    Tab,
    /// Literal text with no newline.
    Literal(String),
}
