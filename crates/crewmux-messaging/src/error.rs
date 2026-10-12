//! Queue and ask failures.

use thiserror::Error;

/// A domain rule rejected a queue, claim, inbox, or ask operation.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum Error {
    /// The item is not in the book.
    #[error("qitem {0} not found")]
    NotFound(String),
    /// A second claimant arrived while the row was in progress.
    #[error("already claimed by {by}")]
    AlreadyClaimed {
        /// Current claimant.
        by: String,
    },
    /// Hot-potato, state, or claim rule. `code` matches the TypeScript name.
    #[error("{message}")]
    Rejected {
        /// Stable code such as `missing_closure_reason`.
        code: &'static str,
        /// Operator text, including the TypeScript sentence where one exists.
        message: String,
    },
    /// A test hook fired before commit. The book is unchanged.
    #[error("crash before commit at {0}")]
    Crashed(&'static str),
}
