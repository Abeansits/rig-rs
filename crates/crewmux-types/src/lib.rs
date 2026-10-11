//! Shared IDs, enums and wire DTOs; no logic.

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
