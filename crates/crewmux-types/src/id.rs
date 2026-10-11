//! Id and name newtypes. Slice 1 may wrap these in ULIDs; the wire form stays a string.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::IdError;

macro_rules! token {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Parse a non-empty token with no path or whitespace characters.
            ///
            /// # Errors
            ///
            /// Returns [`IdError`] when `raw` is empty or contains a reserved character.
            pub fn parse(raw: &str) -> Result<Self, IdError> {
                validate_token(raw)?;
                Ok(Self(raw.to_owned()))
            }

            /// Borrow the token.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

token!(
    /// Seat identity. Slice 1 owns the ULID generator; this slice accepts any safe token.
    SeatId
);
token!(
    /// Rig identity.
    RigId
);
token!(
    /// Pod identity within a rig. Pods are named, not globally unique.
    PodId
);
token!(
    /// tmux session identity for a seat.
    SessionId
);
token!(
    /// Human-readable rig name.
    RigName
);
token!(
    /// Human-readable pod name.
    PodName
);
token!(
    /// Human-readable seat name.
    SeatName
);

fn validate_token(raw: &str) -> Result<(), IdError> {
    if raw.is_empty() || raw.chars().all(char::is_whitespace) {
        return Err(IdError::Empty);
    }
    if raw.chars().any(reserved) {
        return Err(IdError::Reserved);
    }
    Ok(())
}

fn reserved(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '/' | '\\' | '\0')
}
