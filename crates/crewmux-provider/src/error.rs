//! Errors from format-preserving provider writers.

use std::path::PathBuf;

use thiserror::Error;

use crate::drift::DriftReport;

/// Failure while reading or publishing a provider file.
#[derive(Debug, Error)]
pub enum ProviderError {
    /// A Claude process is running, so the user config must not be written.
    #[error("Claude is running (pids {pids:?}); refusing to write the Claude user config")]
    ClaudeRunning {
        /// Process ids reported by the probe.
        pids: Vec<u32>,
    },
    /// The file is not valid for the writer that opened it.
    #[error("failed to parse {}: {msg}", file.display())]
    Parse {
        /// File that could not be parsed.
        file: PathBuf,
        /// Parser message.
        msg: String,
    },
    /// Filesystem failure. The destination is left unchanged when this happens
    /// before a successful rename.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// A compare-and-swap could not confirm the intended bytes.
    #[error("drift in {}", report.file.display())]
    Drift {
        /// What was expected and what was found.
        report: DriftReport,
    },
}
