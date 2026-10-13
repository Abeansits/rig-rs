// Derived from openrig packages/daemon/src/domain/workflow-projector.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Workflow failures. `code` is the TypeScript error name.

use thiserror::Error;

/// A workflow operation failed.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum Error {
    /// Operator or spec rejection.
    #[error("{message}")]
    Rejected {
        /// Stable code such as `exit_not_allowed`.
        code: &'static str,
        /// Operator text.
        message: String,
        /// HTTP status the route should use.
        status: u16,
    },
    /// A crash hook fired. The published engine is unchanged.
    #[error("workflow crashed at {0}")]
    Crashed(&'static str),
}

impl Error {
    pub(crate) fn reject(code: &'static str, message: impl Into<String>, status: u16) -> Self {
        Self::Rejected {
            code,
            message: message.into(),
            status,
        }
    }
}

impl From<crewmux_messaging::Error> for Error {
    fn from(err: crewmux_messaging::Error) -> Self {
        match err {
            crewmux_messaging::Error::Rejected { code, message } => Self::Rejected {
                code,
                message,
                status: 400,
            },
            crewmux_messaging::Error::NotFound(id) => {
                Self::reject("packet_not_found", format!("qitem {id} not found"), 404)
            }
            crewmux_messaging::Error::AlreadyClaimed { by } => {
                Self::reject("already_claimed", format!("already claimed by {by}"), 409)
            }
            crewmux_messaging::Error::Crashed(point) => Self::Crashed(point),
        }
    }
}
