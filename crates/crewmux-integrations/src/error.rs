//! Errors from external terminal launchers.

use std::io;

use thiserror::Error;

/// A Herdr or Ghostty launch failed. The TUI falls back to tmux.
#[derive(Debug, Error)]
pub enum Error {
    /// This platform cannot run the launcher.
    #[error("launcher is not supported on this platform")]
    Unsupported,
    /// The socket or process failed.
    #[error("launcher i/o: {0}")]
    Io(#[source] io::Error),
    /// The peer answered, but not with a usable result.
    #[error("launcher protocol: {0}")]
    Protocol(String),
}
