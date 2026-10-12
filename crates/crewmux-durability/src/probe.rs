//! Claude session jsonl: `~/.claude/projects/<cwd-slug>/<session-id>.jsonl`.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::Error;

/// User-turn text taken from a Claude session file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserTurn {
    /// Extracted text.
    pub text: String,
}

/// Locate the session file. The slug replaces every non-alphanumeric byte with `-`.
#[must_use]
pub fn transcript_path(home: &Path, cwd: &Path, session_id: &str) -> PathBuf {
    home.join(".claude")
        .join("projects")
        .join(project_slug(cwd))
        .join(format!("{session_id}.jsonl"))
}

/// Claude's project directory slug.
#[must_use]
pub fn project_slug(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect()
}

/// User turns whose jsonl bytes start at or after `after_bytes`.
///
/// A missing file is an empty list. `after_bytes` is the length recorded before
/// the paste, so only turns appended after that cursor are returned.
///
/// # Errors
///
/// Returns [`Error::Io`] when the file exists but cannot be read, and
/// [`Error::Parse`] when a line is not JSON.
pub fn user_turns_after(path: &Path, after_bytes: u64) -> Result<Vec<UserTurn>, Error> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    let start = usize::try_from(after_bytes).unwrap_or(bytes.len());
    let start = start.min(bytes.len());
    let tail = skip_partial_line(&bytes, start);
    parse_turns(path, tail)
}

/// Byte length of the transcript. Missing files are `0`.
///
/// # Errors
///
/// Returns [`Error::Io`] when the file exists but cannot be stated.
pub fn transcript_len(path: &Path) -> Result<u64, Error> {
    match std::fs::metadata(path) {
        Ok(meta) => Ok(meta.len()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(err) => Err(err.into()),
    }
}

fn skip_partial_line(bytes: &[u8], start: usize) -> &[u8] {
    let Some(tail) = bytes.get(start..) else {
        return &[];
    };
    if start == 0 {
        return tail;
    }
    let boundary = bytes.get(start.saturating_sub(1)) == Some(&b'\n');
    if boundary {
        return tail;
    }
    let Some(newline) = tail.iter().position(|byte| *byte == b'\n') else {
        return &[];
    };
    tail.get(newline.saturating_add(1)..).unwrap_or(&[])
}

fn parse_turns(path: &Path, tail: &[u8]) -> Result<Vec<UserTurn>, Error> {
    let text = String::from_utf8_lossy(tail);
    let mut turns = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line).map_err(|err| Error::Parse {
            path: path.to_path_buf(),
            line: index.saturating_add(1),
            msg: err.to_string(),
        })?;
        if let Some(text) = user_text(&value) {
            turns.push(UserTurn { text });
        }
    }
    Ok(turns)
}

fn user_text(value: &Value) -> Option<String> {
    let message = value.get("message").unwrap_or(value);
    let role = message.get("role").and_then(Value::as_str);
    let kind = value.get("type").and_then(Value::as_str);
    if role != Some("user") && kind != Some("user") {
        return None;
    }
    text_of(message.get("content").or_else(|| value.get("content"))?)
}

fn text_of(content: &Value) -> Option<String> {
    if let Some(text) = content.as_str() {
        return Some(text.to_owned());
    }
    let items = content.as_array()?;
    let mut parts = Vec::new();
    for item in items {
        if item.get("type").and_then(Value::as_str) == Some("tool_result") {
            continue;
        }
        if let Some(text) = item.get("text").and_then(Value::as_str) {
            parts.push(text.to_owned());
        } else if let Some(text) = item.as_str() {
            parts.push(text.to_owned());
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}
