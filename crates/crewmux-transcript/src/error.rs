//! Failures while reading a provider transcript.

use std::path::PathBuf;

use thiserror::Error;

/// A transcript path or jsonl document could not be read.
#[derive(Debug, Error)]
pub enum Error {
    /// The transcript file or its directory could not be read.
    #[error("transcript io: {0}")]
    Io(#[from] std::io::Error),
    /// A resume token or session id is not a UUID.
    #[error("session id is not a UUID")]
    SessionId,
    /// A jsonl line is not a JSON value.
    #[error("transcript {} line {line}: {msg}", path.display())]
    Parse {
        /// Transcript path.
        path: PathBuf,
        /// 1-based line number.
        line: usize,
        /// Parser message.
        msg: String,
    },
    /// The file has no complete jsonl line yet.
    #[error("transcript {} is still being written", path.display())]
    Incomplete {
        /// Transcript path.
        path: PathBuf,
    },
}
