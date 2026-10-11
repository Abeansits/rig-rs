//! Domain errors.

use thiserror::Error;

/// A lifecycle or repository failure.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    /// The named rig is not in the store.
    #[error("rig not found: {0}")]
    RigNotFound(String),
    /// The named seat is not in the store.
    #[error("seat not found: {0}")]
    SeatNotFound(String),
    /// The caller asked for something the domain rejects.
    #[error("{0}")]
    Invalid(String),
    /// Persistence failed. The store maps its own errors into this.
    #[error("store: {0}")]
    Store(String),
}
