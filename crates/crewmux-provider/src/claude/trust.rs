// Derived from openrig packages/daemon/src/adapters/claude-code-adapter.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
//! Trust pre-accept. This is the only module that writes the Claude user config.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::atomic;
use crate::backup::Backups;
use crate::drift::DriftReport;
use crate::error::ProviderError;
use crate::json_edit::{self, TrustEdit};

const ATTEMPTS: u32 = 8;

/// Basename of the Claude user config. This module is the only writer of that file.
pub(crate) const USER_CONFIG_BASENAME: &str = ".claude.json";

/// Basename of the Claude user config, for the guard test and the writer lint.
#[must_use]
pub fn user_config_basename() -> &'static str {
    USER_CONFIG_BASENAME
}

/// Reports Claude process ids for this user. The real probe reads the process table.
pub trait ProcessProbe {
    /// Claude process ids, or an empty list when none are running.
    fn claude_pids(&self) -> Vec<u32>;
}

/// Guarded writer for the Claude user config (`<home>/.claude.json` in production).
pub struct ClaudeTrust<'a> {
    probe: &'a dyn ProcessProbe,
    path: PathBuf,
    backups: Backups,
}

/// What `pre_accept` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustOutcome {
    /// The trust flag was written.
    Written,
    /// The project was already trusted. The file was not opened for writing.
    AlreadyTrusted,
    /// A Claude process is running. The file was not opened for writing.
    SkippedClaudeRunning,
}

/// Identity of a file, used to prove a skipped write did not replace it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    /// Inode, when the platform reports one.
    pub inode: u64,
    /// Modification time.
    pub modified: Option<SystemTime>,
    /// Length in bytes.
    pub len: u64,
}

impl<'a> ClaudeTrust<'a> {
    /// `path` is the Claude user config. Tests pass a temp file, never the real home.
    #[must_use]
    pub fn new(probe: &'a dyn ProcessProbe, path: impl Into<PathBuf>, backups: Backups) -> Self {
        Self {
            probe,
            path: path.into(),
            backups,
        }
    }

    /// Refuse when the probe reports a Claude process.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::ClaudeRunning`] with the probed pids.
    pub fn reject_if_running(&self) -> Result<(), ProviderError> {
        let pids = self.probe.claude_pids();
        if pids.is_empty() {
            Ok(())
        } else {
            Err(ProviderError::ClaudeRunning { pids })
        }
    }

    /// Set `hasTrustDialogAccepted` for `project` when no Claude process is running.
    ///
    /// The write is compare-and-swap: the bytes are re-read before the rename and
    /// checked again after it, with up to 8 attempts.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Parse`] for a non-object document and
    /// [`ProviderError::Drift`] when every attempt loses the race.
    pub fn pre_accept(&self, project: &Path) -> Result<TrustOutcome, ProviderError> {
        if self.reject_if_running().is_err() {
            return Ok(TrustOutcome::SkippedClaudeRunning);
        }
        let key = project.to_string_lossy();
        for _ in 0..ATTEMPTS {
            if self.reject_if_running().is_err() {
                return Ok(TrustOutcome::SkippedClaudeRunning);
            }
            if let Attempt::Done(outcome) = self.attempt(key.as_ref())? {
                return Ok(outcome);
            }
        }
        let found = read_optional(&self.path)?;
        Err(ProviderError::Drift {
            report: DriftReport {
                file: self.path.clone(),
                expected: "hasTrustDialogAccepted=true".to_owned(),
                found,
            },
        })
    }

    /// Inode, mtime, and length. `None` when the file does not exist.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Io`] when the file exists but cannot be stated.
    pub fn stamp(&self) -> Result<Option<FileStamp>, ProviderError> {
        stamp_of(&self.path)
    }
}

enum Attempt {
    Done(TrustOutcome),
    Retry,
}

impl ClaudeTrust<'_> {
    fn attempt(&self, project: &str) -> Result<Attempt, ProviderError> {
        let before = read_optional(&self.path)?;
        let edited =
            json_edit::set_project_trust(&before, project).map_err(|msg| ProviderError::Parse {
                file: self.path.clone(),
                msg,
            })?;
        let TrustEdit::Changed(next) = edited else {
            return Ok(Attempt::Done(TrustOutcome::AlreadyTrusted));
        };
        self.backups.capture_once(&self.path)?;
        let staged = atomic::stage(&self.path, next.as_bytes(), None)?;
        if read_optional(&self.path)? != before {
            drop(staged);
            return Ok(Attempt::Retry);
        }
        staged.rename()?;
        if read_optional(&self.path)? == next {
            Ok(Attempt::Done(TrustOutcome::Written))
        } else {
            Ok(Attempt::Retry)
        }
    }
}

fn read_optional(path: &Path) -> Result<String, ProviderError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error.into()),
    }
}

fn stamp_of(path: &Path) -> Result<Option<FileStamp>, ProviderError> {
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(Some(FileStamp {
        inode: std::os::unix::fs::MetadataExt::ino(&meta),
        modified: meta.modified().ok(),
        len: meta.len(),
    }))
}
