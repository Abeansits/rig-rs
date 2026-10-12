//! Restore and snapshot failures.

use thiserror::Error;

/// A snapshot or restore plan was refused.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    /// The manifest hash does not match the backup file.
    #[error("snapshot {id} is corrupted: sha256 mismatch (manifest {expected}, file {found})")]
    Corrupt {
        /// Snapshot id.
        id: String,
        /// Hash stored in the manifest.
        expected: String,
        /// Hash of the backup bytes.
        found: String,
    },
    /// The backup was written by a newer build.
    #[error(
        "snapshot schema version {found} is newer than this build supports ({supported}); refusing to restore"
    )]
    NewerSchema {
        /// Version in the manifest or the backup file.
        found: u32,
        /// Migrations this build knows.
        supported: u32,
    },
    /// Launch edges contain a cycle.
    #[error("launch edges cycle; refusing to invent an order")]
    Cycle,
    /// The transcript probe failed.
    #[error("transcript probe: {0}")]
    Probe(String),
    /// A caller-supplied id was not valid.
    #[error("{0}")]
    Id(String),
}
