//! Shared IDs, enums, wire DTOs, and the tmux version parser.
//!
//! [`tmux_version`] is the canonical parser. Slice 1b re-exports it instead of
//! keeping a second copy next to the tmux control modules.

mod doctor;
pub mod tmux_version;

pub use doctor::{CheckStatus, DoctorCheck};
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
