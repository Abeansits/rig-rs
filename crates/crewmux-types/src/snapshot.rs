//! Snapshot and direct-delivery identifiers.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::error::TypeError;

/// Opaque snapshot identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SnapshotId(Ulid);

impl SnapshotId {
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

impl Default for SnapshotId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SnapshotId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for SnapshotId {
    type Err = TypeError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Ulid::from_str(text)
            .map(Self)
            .map_err(|err| TypeError::Id(err.to_string()))
    }
}

/// Direct or startup delivery id. Caller-supplied tokens are allowed.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeliveryId(String);

impl DeliveryId {
    /// Reject an empty id or one that contains whitespace.
    ///
    /// # Errors
    ///
    /// Returns [`TypeError::Name`] when `text` is empty or has whitespace or a control character.
    pub fn new(text: impl Into<String>) -> Result<Self, TypeError> {
        let text = text.into();
        let ok = !text.is_empty() && !text.chars().any(|ch| ch.is_whitespace() || ch.is_control());
        if ok {
            Ok(Self(text))
        } else {
            Err(TypeError::Name(text))
        }
    }

    /// A new `delivery-<ulid>` id.
    #[must_use]
    pub fn generate() -> Self {
        Self(format!("delivery-{}", Ulid::generate()))
    }

    /// Borrow the id.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DeliveryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl AsRef<str> for DeliveryId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for DeliveryId {
    type Err = TypeError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::new(text)
    }
}
