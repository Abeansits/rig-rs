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
    /// The invocation was rejected before any file was written. Exit 1.
    #[error("{0}")]
    Usage(String),
    /// A provider writer failed. Exit 1.
    #[error(transparent)]
    Provider(#[from] crewmux_provider::ProviderError),
    /// Reading or spawning failed. Exit 1.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The daemon returned a body that was not JSON. Exit 1.
    #[error("{0}")]
    Json(String),
    /// Doctor fixture JSON could not be read. Exit 1.
    #[error("doctor fixture: {0}")]
    Fixture(String),
}

impl Error {
    /// Process exit code for this failure.
    #[must_use]
    pub const fn code(&self) -> i32 {
        match self {
            Self::Unreachable(_) => 2,
            Self::User(_)
            | Self::Usage(_)
            | Self::Provider(_)
            | Self::Io(_)
            | Self::Json(_)
            | Self::Fixture(_) => 1,
        }
    }
}
