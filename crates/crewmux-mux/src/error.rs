//! Errors from the multiplexer.

use std::io;

use crate::version::Version;

/// Failure from a multiplexer operation.
#[derive(Debug, thiserror::Error)]
pub enum MuxError {
    /// The named session or pane does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// The caller asked for something tmux must not do.
    #[error("{0}")]
    Forbidden(&'static str),
    /// The installed tmux is older than the supported minimum.
    #[error("tmux {found} is older than minimum {min}")]
    TmuxTooOld {
        /// Version reported by `tmux -V`.
        found: Version,
        /// Oldest version this crate will drive.
        min: Version,
    },
    /// A tmux process exited unsuccessfully.
    #[error("command {argv:?} exited {code:?}: {stderr}")]
    Exec {
        /// Argv that was run, including the tmux binary.
        argv: Vec<String>,
        /// Process exit code, when the OS reported one.
        code: Option<i32>,
        /// Combined stderr (and stdout, when useful).
        stderr: String,
    },
    /// Control-mode protocol or caller-input failure.
    #[error("control mode: {0}")]
    Control(String),
    /// An OS I/O failure outside a tmux exit status.
    #[error(transparent)]
    Io(#[from] io::Error),
}
