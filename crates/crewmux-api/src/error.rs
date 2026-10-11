//! Errors from the typed daemon client.

use std::io;

use thiserror::Error;

/// Failure talking to the daemon over HTTP.
#[derive(Debug, Error)]
pub enum Error {
    /// The request path was not a single absolute HTTP path.
    #[error("daemon HTTP path must be one absolute path")]
    BadPath,
    /// The request method was not an ASCII token.
    #[error("daemon HTTP method must be an ASCII token")]
    BadMethod,
    /// A header name or value contained a line break.
    #[error("daemon HTTP header must be a single line")]
    BadHeader,
    /// The socket or TCP port could not be connected.
    #[error("failed to connect to {target}: {source}")]
    Connect {
        /// Dial target, for the operator message.
        target: String,
        /// OS connect error.
        #[source]
        source: io::Error,
    },
    /// A read or write failed after the connection was up.
    #[error("HTTP exchange with {target} failed: {source}")]
    Exchange {
        /// Dial target, for the operator message.
        target: String,
        /// OS I/O error.
        #[source]
        source: io::Error,
    },
    /// The bytes were not an HTTP/1 response.
    #[error("daemon response was not valid HTTP: {0}")]
    Protocol(String),
    /// The body was not UTF-8.
    #[error("daemon response body was not UTF-8")]
    BodyEncoding,
    /// The status was not 2xx.
    #[error("daemon returned HTTP {status} for {path}")]
    Status {
        /// Request path.
        path: String,
        /// HTTP status.
        status: u16,
    },
    /// The body was not the expected JSON type.
    #[error("daemon JSON for {path} did not match the contract: {message}")]
    Json {
        /// Request path.
        path: String,
        /// serde message.
        message: String,
    },
}
