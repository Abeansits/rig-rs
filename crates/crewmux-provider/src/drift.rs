//! Difference between the bytes a writer expected and the bytes on disk.

use std::path::PathBuf;

/// A doctor-facing description of a file that no longer matches what we wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriftReport {
    /// File that drifted.
    pub file: PathBuf,
    /// Bytes the writer expected to observe.
    pub expected: String,
    /// Bytes found on disk.
    pub found: String,
}
