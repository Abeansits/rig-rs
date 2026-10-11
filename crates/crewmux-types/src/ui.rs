//! View tokens the TUI uses for rows. These are not slice 1's ULID ids.
//!
//! Slice 1 [`crate::SeatId`] is a ULID and [`crate::SeatName`] rejects dots.
//! Explorer rows still address agent names such as `dev50.driver`.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::TypeError;

/// A seat row key. Not a [`crate::SeatId`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SeatKey(String);

impl SeatKey {
    /// Reject an empty token or one that would break a path.
    ///
    /// # Errors
    ///
    /// Returns [`TypeError::Name`] when `text` is empty or contains whitespace or a slash.
    pub fn parse(text: impl Into<String>) -> Result<Self, TypeError> {
        validate_token(text.into()).map(Self)
    }

    /// Borrow the token.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SeatKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Pod label in the explorer. Slice 1 seats have no pod column.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PodName(String);

impl PodName {
    /// Reject an empty token or one that would break a path.
    ///
    /// # Errors
    ///
    /// Returns [`TypeError::Name`] when `text` is empty or contains whitespace or a slash.
    pub fn parse(text: impl Into<String>) -> Result<Self, TypeError> {
        validate_token(text.into()).map(Self)
    }

    /// Borrow the name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PodName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// tmux session name passed to `-t`. Not a [`crate::SessionId`] ULID.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TmuxSession(String);

impl TmuxSession {
    /// Reject an empty token or one that would break a path.
    ///
    /// # Errors
    ///
    /// Returns [`TypeError::Name`] when `text` is empty or contains whitespace or a slash.
    pub fn parse(text: impl Into<String>) -> Result<Self, TypeError> {
        validate_token(text.into()).map(Self)
    }

    /// Borrow the session name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TmuxSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Which external launcher opens a seat when tmux would otherwise nest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LauncherKind {
    /// Herdr control socket. Attach-only: never start or stop the Herdr server.
    Herdr,
    /// Ghostty window via `osascript` on macOS.
    Ghostty,
}

impl LauncherKind {
    /// Parse `herdr` or `ghostty`.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "herdr" => Some(Self::Herdr),
            "ghostty" => Some(Self::Ghostty),
            _ => None,
        }
    }
}

fn validate_token(text: String) -> Result<String, TypeError> {
    let bad = text.is_empty()
        || text
            .chars()
            .any(|ch| ch.is_whitespace() || matches!(ch, '/' | '\\' | '\0'));
    if bad {
        Err(TypeError::Name(text))
    } else {
        Ok(text)
    }
}
