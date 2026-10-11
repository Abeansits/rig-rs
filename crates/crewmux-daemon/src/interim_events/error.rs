//! Errors for the temporary event stream. Slice 4 deletes this with the module.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// Failure while binding or serving the event stream.
#[derive(Debug, Error)]
pub enum Error {
    /// The socket path does not fit in `sun_path`.
    #[error("socket path is {bytes} bytes; the sun_path guard is {limit}: {path}")]
    SunPath {
        /// Path that was rejected.
        path: PathBuf,
        /// Byte length.
        bytes: usize,
        /// Guard.
        limit: usize,
    },
    /// Creating the runtime directory or binding the socket failed.
    #[error("event socket {path}: {source}")]
    Bind {
        /// Socket path.
        path: PathBuf,
        /// OS error.
        #[source]
        source: io::Error,
    },
    /// axum stopped with an error.
    #[error("event server failed: {0}")]
    Serve(#[source] io::Error),
}
