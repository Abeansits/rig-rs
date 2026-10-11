//! CLI failures and the process exit codes they map to.

use thiserror::Error;

/// A failure the `rig` binary turns into an exit code.
#[derive(Debug, Error)]
pub enum Error {
    /// The user passed a bad spec, seat, or flag. Exit 1.
    #[error("{0}")]
    User(String),
    /// The daemon could not be started or reached. Exit 2.
    #[error("{0}")]
    Unreachable(String),
    /// Reading or spawning failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The daemon returned a body that was not JSON.
    #[error("{0}")]
    Json(String),
}

impl Error {
    /// Process exit code for this failure.
    #[must_use]
    pub const fn code(&self) -> i32 {
        match self {
            Self::Unreachable(_) => 2,
            Self::User(_) | Self::Io(_) | Self::Json(_) => 1,
        }
    }
}
