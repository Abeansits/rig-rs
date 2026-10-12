//! Durability failures.

use thiserror::Error;

/// A transcript probe or rollout read failed.
#[derive(Debug, Error)]
pub enum Error {
    /// The rollout file could not be read as Codex metadata.
    #[error("{0}")]
    Message(String),
}
