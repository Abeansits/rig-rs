//! Errors returned by the `rig` CLI library.

use thiserror::Error;

/// Failure while parsing arguments or applying a setup plan.
#[derive(Debug, Error)]
pub enum Error {
    /// The invocation was rejected before any file was written.
    #[error("{0}")]
    Usage(String),
    /// A provider writer failed.
    #[error(transparent)]
    Provider(#[from] crewmux_provider::ProviderError),
    /// A filesystem call failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Doctor fixture JSON could not be read.
    #[error("doctor fixture: {0}")]
    Fixture(String),
}
