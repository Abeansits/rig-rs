//! Shared IDs, enums, wire DTOs, and the tmux version parser.
//!
//! [`Key`] is the multiplexer key enum. Seat ids, runtimes, and readiness
//! codes are the slice 1 vocabulary. [`tmux_version`] is the canonical parser;
//! slice 1b re-exports it from `crewmux-mux`.

mod chat;
mod doctor;
mod error;
mod id;
mod readiness;
mod runtime;
mod ui;

pub mod tmux_version;

pub use chat::{
    ChatCursor, ChatId, MAX_CURSOR_LEN, MAX_SEAT_TOKEN_LEN, MAX_SENDER_LEN, MAX_TOPIC_LEN,
    SEAT_ID_ENV, SEAT_ID_HEADER, SEAT_TOKEN_ENV, SEAT_TOKEN_HEADER, SenderRef, TopicId,
};
pub use doctor::{CheckStatus, DoctorCheck};
pub use error::TypeError;
pub use id::{MAX_NAME_LEN, QItemId, RigId, RigName, SeatId, SeatName, SessionId};
pub use readiness::ReadinessCode;
pub use runtime::{ForkSource, LaunchPosture, RuntimeKind};
pub use tmux_version::{Version, parse_version};
pub use ui::{LauncherKind, PodName, SeatKey, TmuxSession};

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
    /// Kill the current line. Clears a paste that failed verification.
    CtrlU,
    /// Drop a whole draft and redraw the prompt. Used when line clears cannot
    /// bring a tall buffer paste back to an empty composer.
    CtrlG,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Tab.
    Tab,
    /// Literal text with no newline.
    Literal(String),
}
