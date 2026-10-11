//! Terminal UI errors.

use std::io;

use thiserror::Error;

/// Failure while the TUI is running.
#[derive(Debug, Error)]
pub enum Error {
    /// The terminal could not be switched into or out of the TUI modes.
    #[error("terminal mode: {0}")]
    Terminal(#[source] io::Error),
    /// The control socket could not be bound or a client line failed.
    #[error("control socket: {0}")]
    Socket(#[source] io::Error),
    /// The tokio runtime could not start.
    #[error("runtime: {0}")]
    Runtime(#[source] io::Error),
}

impl Error {
    /// Socket failure from a display message.
    #[must_use]
    pub fn socket(message: impl Into<String>) -> Self {
        Self::Socket(io::Error::other(message.into()))
    }
}
