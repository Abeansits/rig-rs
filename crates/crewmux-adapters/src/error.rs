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
    /// The pane is not a ready composer, so nothing was pasted.
    #[error("seat is not ready for input")]
    NotReady,
    /// The composer after the paste was not the payload. Enter was not sent.
    #[error("composer did not show the paste")]
    PasteMismatch,
    /// The transcript did not show exactly one matching new user turn.
    #[error("transcript did not confirm the delivery")]
    NotVerified,
    /// The new user turn merged the paste with other text.
    #[error("transcript turn merged the paste with other text")]
    Contaminated,
    /// A per-seat provider file could not be written.
    #[error(transparent)]
    Provider(#[from] crewmux_provider::ProviderError),
}
