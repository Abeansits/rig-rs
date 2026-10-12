//! Seat or operator identity for tools that deliver text.

use crewmux_types::{SEAT_ID_ENV, SEAT_TOKEN_ENV, SeatId, SeatToken};

use crate::error::Error;

/// Who a tool claims to be when it talks to the daemon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Caller {
    /// Both seat variables were set and parsed.
    Seat {
        /// Seat id from [`SEAT_ID_ENV`].
        id: SeatId,
        /// Seat secret from [`SEAT_TOKEN_ENV`].
        token: SeatToken,
    },
    /// Neither seat variable was set.
    Operator,
}

impl Caller {
    /// Read [`SEAT_ID_ENV`] and [`SEAT_TOKEN_ENV`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::PartialSeatEnv`] when only one variable is set, and
    /// [`Error::Seat`] when a value is not a seat id or token.
    pub fn from_env() -> Result<Self, Error> {
        let id = std::env::var(SEAT_ID_ENV).ok();
        let token = std::env::var(SEAT_TOKEN_ENV).ok();
        caller_from_pair(id.as_deref(), token.as_deref())
    }

    /// `X-Rig-Seat-Id` and `X-Rig-Seat-Token` when this is a seat.
    #[must_use]
    pub fn headers(&self) -> Vec<(String, String)> {
        match self {
            Self::Seat { id, token } => vec![
                (crewmux_types::SEAT_ID_HEADER.to_owned(), id.to_string()),
                (
                    crewmux_types::SEAT_TOKEN_HEADER.to_owned(),
                    token.as_str().to_owned(),
                ),
            ],
            Self::Operator => Vec::new(),
        }
    }
}

/// Build a caller from an explicit pair. Tests use this instead of the process environment.
///
/// # Errors
///
/// Same as [`Caller::from_env`].
pub fn caller_from_pair(id: Option<&str>, token: Option<&str>) -> Result<Caller, Error> {
    match (id, token) {
        (None, None) => Ok(Caller::Operator),
        (Some(id), Some(token)) => Ok(Caller::Seat {
            id: id.parse()?,
            token: SeatToken::new(token)?,
        }),
        (Some(_), None) | (None, Some(_)) => Err(Error::PartialSeatEnv),
    }
}
