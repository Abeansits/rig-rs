//! Codex rollout user turns for ADR-0009 delivery matching.

use std::path::{Path, PathBuf};

use crewmux_adapters::codex::{same_delivery, user_turn_texts};
use crewmux_types::RuntimeKind;

use crate::error::Error;

/// User items in one Codex rollout file.
#[derive(Clone, Debug)]
pub struct CodexRolloutProbe {
    path: PathBuf,
}

impl CodexRolloutProbe {
    /// Probe `path`. The file is read on each query.
    #[must_use]
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Path passed to [`open`](Self::open).
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// User-role texts in rollout order.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the file is missing or not a rollout.
    pub fn user_turns(&self) -> Result<Vec<String>, Error> {
        user_turn_texts(&self.path).map_err(|err| Error::Message(err.to_string()))
    }

    /// Whether a user item has the same Codex delivery fingerprint as `sent`.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the rollout cannot be read.
    pub fn matches_sent(&self, sent: &str) -> Result<bool, Error> {
        let turns = self.user_turns()?;
        Ok(turns
            .iter()
            .any(|turn| same_delivery(RuntimeKind::Codex, sent, turn)))
    }
}
