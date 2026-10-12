//! Transcript cursor and the post-submit delivery check.

use std::path::Path;

use crewmux_types::RuntimeKind;

use crate::error::Error;
use crate::normalize::{TurnMatch, match_turn};

/// Byte length of a transcript before a paste. A missing file is `0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TranscriptCursor(pub u64);

/// One user turn read from a provider transcript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserTurn {
    /// Extracted text.
    pub text: String,
    /// Byte offset of the jsonl line that held this turn.
    pub at: TranscriptCursor,
}

/// A provider transcript the submit path can read.
pub trait TranscriptSource: Send + Sync {
    /// Runtime that stored the turns.
    fn runtime(&self) -> RuntimeKind;

    /// Length before the paste. A missing file is [`TranscriptCursor`]`(0)`.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the file exists but cannot be stated.
    fn cursor(&self) -> Result<TranscriptCursor, Error>;

    /// User turns whose bytes start at or after `after`.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the transcript cannot be parsed.
    fn user_turns_after(&self, after: TranscriptCursor) -> Result<Vec<UserTurn>, Error>;
}

/// Exactly one new matching user turn, or why that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// One new user turn matches the paste, and it is the only new turn.
    Landed,
    /// No matching user turn has been appended yet.
    Missing,
    /// The new turn merged the paste with other text.
    Composite,
    /// More than one matching user turn was appended.
    Duplicate(usize),
}

/// Compare turns after `since` with `sent`.
///
/// Exactly one [`TurnMatch::Equal`] turn, and no other new turns, is
/// [`Delivery::Landed`]. Any composite turn is [`Delivery::Composite`].
///
/// # Errors
///
/// Returns [`Error`] when the transcript cannot be read.
pub fn verify_delivery(
    source: &dyn TranscriptSource,
    since: TranscriptCursor,
    sent: &str,
) -> Result<Delivery, Error> {
    let turns = source.user_turns_after(since)?;
    let runtime = source.runtime();
    let mut equal = 0usize;
    let mut composite = false;
    for turn in &turns {
        match match_turn(runtime, sent, &turn.text) {
            TurnMatch::Equal => equal = equal.saturating_add(1),
            TurnMatch::Composite => composite = true,
            TurnMatch::Different => {}
        }
    }
    if composite {
        return Ok(Delivery::Composite);
    }
    if equal == 1 && turns.len() == 1 {
        return Ok(Delivery::Landed);
    }
    if equal > 1 {
        return Ok(Delivery::Duplicate(equal));
    }
    Ok(Delivery::Missing)
}

/// User turns in `path` whose jsonl bytes start at or after `after_bytes`.
///
/// A missing file is an empty list. A last line with no newline is still being
/// written and is skipped.
///
/// # Errors
///
/// Returns [`Error::Io`] when the file exists but cannot be read, and
/// [`Error::Parse`] when a complete line is not JSON.
pub fn user_turns_after(path: &Path, after_bytes: u64) -> Result<Vec<UserTurn>, Error> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    let start = usize::try_from(after_bytes).unwrap_or(bytes.len());
    let start = start.min(bytes.len());
    let tail = skip_partial_line(&bytes, start);
    let tail = drop_incomplete_tail(tail);
    parse_turns(path, tail, start)
}

/// Byte length of the transcript. Missing files are `0`.
///
/// # Errors
///
/// Returns [`Error::Io`] when the file exists but cannot be stated.
pub(crate) fn transcript_len(path: &Path) -> Result<u64, Error> {
    match std::fs::metadata(path) {
        Ok(meta) => Ok(meta.len()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(err) => Err(err.into()),
    }
}

fn drop_incomplete_tail(tail: &[u8]) -> &[u8] {
    if tail.is_empty() || tail.last() == Some(&b'\n') {
        return tail;
    }
    match tail.iter().rposition(|byte| *byte == b'\n') {
        Some(index) => tail.get(..=index).unwrap_or(tail),
        None => &[],
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

fn parse_turns(path: &Path, tail: &[u8], start: usize) -> Result<Vec<UserTurn>, Error> {
    let text = String::from_utf8_lossy(tail);
    let mut turns = Vec::new();
    let mut offset = start;
    for (index, line) in text.lines().enumerate() {
        let line_at = offset;
        offset = offset.saturating_add(line.len()).saturating_add(1);
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line).map_err(|err| Error::Parse {
            path: path.to_path_buf(),
            line: index.saturating_add(1),
            msg: err.to_string(),
        })?;
        if let Some(text) = user_text(&value) {
            if ignored_compact_record(&value, &text) {
                continue;
            }
            turns.push(UserTurn {
                text,
                at: TranscriptCursor(u64::try_from(line_at).unwrap_or(u64::MAX)),
            });
        }
    }
    Ok(turns)
}

fn ignored_compact_record(value: &serde_json::Value, text: &str) -> bool {
    compact_record(value) || command_envelope(text.trim())
}

/// Structure, not a substring. A prose turn that quotes `<local-command-stdout>` stays.
fn compact_record(value: &serde_json::Value) -> bool {
    flag(value, "isCompactSummary")
        || flag(value, "isMeta")
        || value
            .get("message")
            .is_some_and(|message| flag(message, "isMeta"))
        || matches!(
            value.get("subtype").and_then(serde_json::Value::as_str),
            Some("compact_boundary" | "local_command")
        )
}

fn flag(value: &serde_json::Value, key: &str) -> bool {
    value.get(key).and_then(serde_json::Value::as_bool) == Some(true)
}

/// The whole turn is only `command-name` / `command-args` / `command-message` /
/// `command-contents` tags. Anything else, including a quoted tag, is a user turn.
fn command_envelope(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let mut rest = text;
    let mut saw = false;
    while !rest.is_empty() {
        let Some(next) = one_command_tag(rest) else {
            return false;
        };
        saw = true;
        rest = next.trim_start();
    }
    saw
}

fn one_command_tag(text: &str) -> Option<&str> {
    let name = tag_name(text)?;
    if !is_command_tag(name) {
        return None;
    }
    let open_end = text.find('>')?;
    let body = text.get(open_end.saturating_add(1)..)?;
    let close = format!("</{name}>");
    let at = body.find(&close)?;
    body.get(at.saturating_add(close.len())..)
}

fn tag_name(text: &str) -> Option<&str> {
    let rest = text.strip_prefix('<')?;
    let end = rest.find(['>', ' ', '\t', '\n', '\r'])?;
    rest.get(..end)
}

fn is_command_tag(name: &str) -> bool {
    matches!(
        name,
        "command-name" | "command-args" | "command-message" | "command-contents"
    )
}

fn user_text(value: &serde_json::Value) -> Option<String> {
    let message = value.get("message").unwrap_or(value);
    let role = message.get("role").and_then(serde_json::Value::as_str);
    let kind = value.get("type").and_then(serde_json::Value::as_str);
    if role != Some("user") && kind != Some("user") {
        return None;
    }
    text_of(message.get("content").or_else(|| value.get("content"))?)
}

fn text_of(content: &serde_json::Value) -> Option<String> {
    if let Some(text) = content.as_str() {
        return Some(text.to_owned());
    }
    let items = content.as_array()?;
    let mut parts = Vec::new();
    for item in items {
        if item.get("type").and_then(serde_json::Value::as_str) == Some("tool_result") {
            continue;
        }
        if let Some(text) = item.get("text").and_then(serde_json::Value::as_str) {
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
