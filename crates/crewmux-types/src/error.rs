//! Errors from parsing ids and names.

use thiserror::Error;

/// A newtype could not be built.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TypeError {
    /// The text was not a ULID.
    #[error("invalid id: {0}")]
    Id(String),
    /// The name was empty or contained a control character.
    #[error("invalid name: {0}")]
    Name(String),
}
