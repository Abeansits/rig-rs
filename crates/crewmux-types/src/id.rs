//! ULID newtypes for rigs, seats, and sessions, plus name newtypes.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::error::TypeError;

/// Opaque rig identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RigId(Ulid);

impl RigId {
    /// A new random id.
    #[must_use]
    pub fn new() -> Self {
        Self(Ulid::generate())
    }

    /// The underlying ULID.
    #[must_use]
    pub const fn as_ulid(self) -> Ulid {
        self.0
    }
}

impl Default for RigId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for RigId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for RigId {
    type Err = TypeError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Ulid::from_str(text)
            .map(Self)
            .map_err(|err| TypeError::Id(err.to_string()))
    }
}

/// Opaque seat identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SeatId(Ulid);

impl SeatId {
    /// A new random id.
    #[must_use]
    pub fn new() -> Self {
        Self(Ulid::generate())
    }

    /// The underlying ULID.
    #[must_use]
    pub const fn as_ulid(self) -> Ulid {
        self.0
    }
}

impl Default for SeatId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SeatId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for SeatId {
    type Err = TypeError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Ulid::from_str(text)
            .map(Self)
            .map_err(|err| TypeError::Id(err.to_string()))
    }
}

/// Opaque session identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(Ulid);

impl SessionId {
    /// A new random id.
    #[must_use]
    pub fn new() -> Self {
        Self(Ulid::generate())
    }

    /// The underlying ULID.
    #[must_use]
    pub const fn as_ulid(self) -> Ulid {
        self.0
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for SessionId {
    type Err = TypeError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Ulid::from_str(text)
            .map(Self)
            .map_err(|err| TypeError::Id(err.to_string()))
    }
}

/// Longest rig or seat name, in bytes. A seat name is also the chat sender.
pub const MAX_NAME_LEN: usize = 256;

/// Rig name as written in a spec. Not an id.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RigName(String);

impl RigName {
    /// Reject an empty name.
    ///
    /// # Errors
    ///
    /// Returns [`TypeError::Name`] when `text` is empty or contains a newline.
    pub fn new(text: impl Into<String>) -> Result<Self, TypeError> {
        validate_name(text.into()).map(Self)
    }

    /// Borrow the name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RigName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl AsRef<str> for RigName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Seat name as written in a spec. Not an id.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SeatName(String);

impl SeatName {
    /// Reject a name tmux would format-expand.
    ///
    /// # Errors
    ///
    /// Returns [`TypeError::Name`] when `text` is empty or contains a character
    /// outside `[A-Za-z0-9_-]`.
    pub fn new(text: impl Into<String>) -> Result<Self, TypeError> {
        validate_seat_name(text.into()).map(Self)
    }

    /// Borrow the name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SeatName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl AsRef<str> for SeatName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

fn validate_name(text: String) -> Result<String, TypeError> {
    if text.is_empty() || text.len() > MAX_NAME_LEN || text.chars().any(char::is_control) {
        return Err(TypeError::Name(text));
    }
    Ok(text)
}

fn validate_seat_name(text: String) -> Result<String, TypeError> {
    let ok = !text.is_empty()
        && text.len() <= MAX_NAME_LEN
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
    if ok {
        Ok(text)
    } else {
        Err(TypeError::Name(text))
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_NAME_LEN, RigName, SeatName};

    #[test]
    fn names_accept_256_bytes_and_reject_257() {
        let ok = "a".repeat(MAX_NAME_LEN);
        assert_eq!(RigName::new(&ok).expect("rig").as_str(), ok);
        assert_eq!(SeatName::new(&ok).expect("seat").as_str(), ok);
        let over = "a".repeat(MAX_NAME_LEN.saturating_add(1));
        assert!(RigName::new(&over).is_err());
        assert!(SeatName::new(over).is_err());
    }
}
