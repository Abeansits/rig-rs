//! Errors for id and name newtypes.

use thiserror::Error;

/// A newtype rejected an empty or unsafe token.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdError {
    /// The token was empty or only whitespace.
    #[error("id is empty")]
    Empty,
    /// The token contained a character that would break a path or a socket name.
    #[error("id contains a reserved character")]
    Reserved,
}
