//! Errors from building the server or talking to the daemon.

use thiserror::Error;

use crate::names::RIG_MCP_NAME;

/// A failure before a tool result is produced.
#[derive(Debug, Error)]
pub enum Error {
    /// One of the seat variables was set and the other was not.
    #[error("set both RIG_SEAT_ID and RIG_SEAT_TOKEN, or neither")]
    PartialSeatEnv,
    /// The seat id or token was not a valid token.
    #[error(transparent)]
    Seat(#[from] crewmux_types::TypeError),
    /// The daemon client rejected the request or the socket.
    #[error(transparent)]
    Api(#[from] crewmux_api::Error),
    /// A spec file could not be read.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The stdio server stopped.
    #[error("{RIG_MCP_NAME} mcp server stopped: {0}")]
    Serve(String),
    /// A join handle or JSON encode failed.
    #[error("{0}")]
    Message(String),
}
