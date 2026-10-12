//! Chat repository failures.

use thiserror::Error;

/// A chat write or history query failed.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ChatError {
    /// `since` was not a timestamp this crate compares.
    #[error("since is not a timestamp: {0}")]
    UnparseableSince(String),
    /// No topic marker has this name.
    #[error("unknown topic: {0}")]
    UnknownTopic(String),
    /// The caller presented a seat id or a sender claim that did not authenticate.
    #[error("unknown sender")]
    UnknownSender,
    /// The rig id is not in the store.
    #[error("rig not found")]
    UnknownRig,
    /// Persistence failed.
    #[error("store: {0}")]
    Store(String),
}
