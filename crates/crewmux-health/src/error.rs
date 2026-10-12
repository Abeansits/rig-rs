//! Failures from compaction parsing and marker encoding.

use thiserror::Error;

/// A compaction control failure.
#[derive(Debug, Error)]
pub enum Error {
    /// Hook stdin was not a JSON value.
    #[error("hook json: {0}")]
    Hook(String),
    /// A marker or prompt could not be encoded.
    #[error("{0}")]
    Message(String),
}
