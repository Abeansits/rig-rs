//! Daemon failures.

use thiserror::Error;

/// A failure while serving or launching.
#[derive(Debug, Error)]
pub enum Error {
    /// Persistence failed.
    #[error(transparent)]
    Store(#[from] crewmux_store::Error),
    /// A domain rule rejected the change.
    #[error(transparent)]
    Domain(#[from] crewmux_core::Error),
    /// The multiplexer failed.
    #[error(transparent)]
    Mux(#[from] crewmux_mux::MuxError),
    /// An adapter failed.
    #[error(transparent)]
    Adapter(#[from] crewmux_adapters::Error),
    /// The spec YAML was rejected.
    #[error(transparent)]
    Spec(#[from] crewmux_spec::SpecError),
    /// A directory or socket could not be created.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The caller asked for something this slice does not do.
    #[error("{0}")]
    Message(String),
}
