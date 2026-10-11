//! Shared IDs, enums, wire DTOs, and the tmux version parser.
//!
//! [`Key`] is the multiplexer key enum. Seat ids, runtimes, and readiness
//! codes are the slice 1 vocabulary. [`tmux_version`] is the canonical parser;
//! slice 1b re-exports it from `crewmux-mux`.

mod doctor;
mod error;
mod id;
mod readiness;
mod runtime;

pub mod tmux_version;

pub use doctor::{CheckStatus, DoctorCheck};
pub use error::TypeError;
pub use id::{RigId, RigName, SeatId, SeatName, SessionId};
pub use readiness::ReadinessCode;
pub use runtime::{ForkSource, LaunchPosture, RuntimeKind};
pub use tmux_version::{Version, parse_version};

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
