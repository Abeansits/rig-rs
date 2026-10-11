//! Errors for the diff harness and the parity ledger.

use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;

use thiserror::Error;

use crate::diff::PeId;

/// Failure while loading or running the harness, or while reading the ledger.
#[derive(Debug, Error)]
pub enum Error {
    /// Operator-facing text that does not fit a more specific variant.
    #[error("{0}")]
    Message(String),
    /// The pinned TypeScript tree is absent or not v0.6.9. This is not a skip.
    #[error(
        "TS OpenRig v0.6.9 is not installed. The diff harness will not skip this run.\n{detail}"
    )]
    TsMissing {
        /// How to install the pinned tree.
        detail: String,
    },
    /// A rig-rs binding used TCP. Rejected before any connection.
    #[error(
        "the rig-rs side rejects TCP {addr} at load time; use the Unix socket at $RIG_HOME/run/daemon.sock"
    )]
    RsTcpRejected {
        /// Address that was refused.
        addr: SocketAddr,
    },
    /// A TypeScript binding used a Unix socket.
    #[error("the TypeScript side uses TCP 127.0.0.1, not a Unix socket")]
    TsRequiresTcp,
    /// `expected_diff` named an id that is not a heading in the exceptions file.
    #[error("expected diff {pe} is not a heading in {}", file.display())]
    UnknownPe {
        /// Id that was requested.
        pe: PeId,
        /// Exceptions file that was searched.
        file: PathBuf,
    },
    /// The marked exception no longer describes this case.
    #[error("stale exception: case {case} is marked {pe} but {detail}")]
    Stale {
        /// Case id.
        case: String,
        /// Exception id that is no longer needed.
        pe: PeId,
        /// Why the exception does not apply (`the outputs are equal`, or the
        /// Rust side has left the committed snapshot).
        detail: String,
    },
    /// Inherited `OPENRIG_*` / `RIGGED_*` daemon target.
    #[error(
        "hermetic env refused: ambient daemon target {name}={value} is present — the harness will not run against a daemon it did not create"
    )]
    ForeignDaemon {
        /// Variable name.
        name: String,
        /// Variable value.
        value: String,
    },
    /// Inherited injected clock.
    #[error(
        "hermetic env refused: ambient injected-clock {name}={value} is present — unset it before running the harness"
    )]
    AmbientClock {
        /// Variable name.
        name: String,
        /// Variable value.
        value: String,
    },
    /// Inherited tmux client attachment.
    #[error(
        "hermetic env refused: ambient tmux attachment {name}={value} is present — unset {name} so seats cannot reach the default tmux server"
    )]
    AmbientTmux {
        /// Variable name.
        name: String,
        /// Variable value.
        value: String,
    },
    /// Filesystem failure with context.
    #[error("{context}: {source}")]
    Io {
        /// What was being read or written.
        context: String,
        /// OS error.
        #[source]
        source: io::Error,
    },
    /// JSON failure with context.
    #[error("{context}: {source}")]
    Json {
        /// What was being parsed or written.
        context: String,
        /// Serde error.
        #[source]
        source: serde_json::Error,
    },
    /// A normalisation pattern failed to compile.
    #[error("invalid normalisation pattern: {0}")]
    Pattern(String),
    /// A parity-exception id was not `PE-` plus uppercase letters or digits.
    #[error("invalid parity-exception id {0}")]
    BadPe(String),
    /// The xtask argv was not understood.
    #[error("{0}")]
    Usage(String),
}

/// Attach context to an [`io::Error`].
pub fn io_err(context: impl Into<String>, source: io::Error) -> Error {
    Error::Io {
        context: context.into(),
        source,
    }
}
