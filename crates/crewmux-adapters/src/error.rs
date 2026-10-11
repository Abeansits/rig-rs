//! Adapter failures.

use thiserror::Error;

/// A launch, readiness, or stub-agent failure.
#[derive(Debug, Error)]
pub enum Error {
    /// The multiplexer rejected the call.
    #[error(transparent)]
    Mux(#[from] crewmux_mux::MuxError),
    /// The stub script or another caller input is invalid.
    #[error("{0}")]
    Message(String),
    /// Reading or writing the stub process failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
