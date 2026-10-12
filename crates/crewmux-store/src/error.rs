//! Persistence errors.

use thiserror::Error;

/// A failure to open the database or to run a job on the writer thread.
#[derive(Debug, Error)]
pub enum Error {
    /// The writer thread stopped.
    #[error("store closed")]
    Closed,
    /// The file is from a newer build.
    #[error(
        "database schema version {found} is newer than this build supports ({supported}); refusing to open it"
    )]
    NewerSchema {
        /// `PRAGMA user_version` in the file.
        found: usize,
        /// Number of migrations this build knows.
        supported: usize,
    },
    /// SQLite or migration failure.
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    /// Migration runner failure.
    #[error(transparent)]
    Migration(#[from] rusqlite_migration::Error),
    /// Creating the parent directory failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// A domain error raised inside a write.
    #[error(transparent)]
    Domain(#[from] crewmux_core::Error),
    /// JSON stored in the event log could not be read.
    #[error("event json: {0}")]
    Event(String),
    /// A test crash hook rolled the transaction back.
    #[error("crash hook {0}")]
    CrashHook(&'static str),
    /// A chat write or history query failed.
    #[error(transparent)]
    Chat(#[from] crewmux_core::chat::ChatError),
    /// The snapshot backup could not run on the maintenance connection.
    #[error("{0}")]
    Backup(String),
}

impl Error {
    /// Unwrap a chat failure, or wrap any other store failure as [`crewmux_core::chat::ChatError::Store`].
    #[must_use]
    pub fn into_chat(self) -> crewmux_core::chat::ChatError {
        let rendered = self.to_string();
        match self {
            Self::Chat(err) => err,
            Self::Closed
            | Self::NewerSchema { .. }
            | Self::Sqlite(_)
            | Self::Migration(_)
            | Self::Io(_)
            | Self::Domain(_)
            | Self::Event(_)
            | Self::CrashHook(_)
            | Self::Backup(_) => crewmux_core::chat::ChatError::Store(rendered),
        }
    }
}
