//! Claude session jsonl: `~/.claude/projects/<cwd-slug>/<session-id>.jsonl`.

use std::path::{Path, PathBuf};

use crewmux_types::RuntimeKind;

use crate::error::Error;
use crate::source::{
    TranscriptCursor, TranscriptSource, UserTurn, transcript_len, user_turns_after,
};

/// Claude's project directory slug. Every non-alphanumeric byte becomes `-`.
#[must_use]
pub fn project_slug(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect()
}

/// Canonical hyphenated form of a session id.
///
/// # Errors
///
/// Returns [`Error::SessionId`] when `session_id` is not a UUID. Braced,
/// `urn:uuid:`, and hyphen-less forms are accepted and stored hyphenated.
pub fn canonical_session_id(session_id: &str) -> Result<String, Error> {
    let id = uuid::Uuid::parse_str(session_id.trim()).map_err(|_| Error::SessionId)?;
    Ok(id.hyphenated().to_string())
}

/// Locate the session file. The id is canonicalised before it is joined.
///
/// # Errors
///
/// Returns [`Error::SessionId`] when `session_id` is not a UUID.
pub fn transcript_path(home: &Path, cwd: &Path, session_id: &str) -> Result<PathBuf, Error> {
    let id = canonical_session_id(session_id)?;
    Ok(home
        .join(".claude")
        .join("projects")
        .join(project_slug(cwd))
        .join(format!("{id}.jsonl")))
}

/// One Claude session file.
#[derive(Clone, Debug)]
pub struct ClaudeTranscript {
    path: PathBuf,
}

impl ClaudeTranscript {
    /// Open `~/.claude/projects/<slug>/<canonical-uuid>.jsonl`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::SessionId`] when `session_id` is not a UUID.
    pub fn open(home: &Path, cwd: &Path, session_id: &str) -> Result<Self, Error> {
        Ok(Self {
            path: transcript_path(home, cwd, session_id)?,
        })
    }

    /// Path of the jsonl file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl TranscriptSource for ClaudeTranscript {
    fn runtime(&self) -> RuntimeKind {
        RuntimeKind::ClaudeCode
    }

    fn cursor(&self) -> Result<TranscriptCursor, Error> {
        Ok(TranscriptCursor(transcript_len(&self.path)?))
    }

    fn user_turns_after(&self, after: TranscriptCursor) -> Result<Vec<UserTurn>, Error> {
        user_turns_after(&self.path, after.0)
    }
}
