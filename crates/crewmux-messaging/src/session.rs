//! Session addresses (`member@rig`) and the human-seat predicate.

use serde::{Deserialize, Serialize};

use crate::error::Error;

/// A session address. Not a ULID. Human seats match `human[-name]@(kernel|host)`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionRef(String);

impl SessionRef {
    /// Reject an empty address.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Rejected`] when `text` is empty or contains whitespace.
    pub fn new(text: impl Into<String>) -> Result<Self, Error> {
        let text = text.into();
        if text.is_empty() || text.chars().any(|ch| ch.is_whitespace() || ch.is_control()) {
            return Err(Error::Rejected {
                code: "invalid_session",
                message: format!("invalid session `{text}`"),
            });
        }
        Ok(Self(text))
    }

    /// Borrow the address.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// TypeScript `isHumanSeatSession`.
    #[must_use]
    pub fn is_human_seat(&self) -> bool {
        is_human_seat(self.as_str())
    }
}

impl std::fmt::Display for SessionRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// `^human(?:-[A-Za-z0-9._-]+)?@(kernel|host)$`.
#[must_use]
pub fn is_human_seat(value: &str) -> bool {
    let Some((name, host)) = value.split_once('@') else {
        return false;
    };
    if host != "kernel" && host != "host" {
        return false;
    }
    if name == "human" {
        return true;
    }
    let Some(rest) = name.strip_prefix("human-") else {
        return false;
    };
    !rest.is_empty()
        && rest
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}
