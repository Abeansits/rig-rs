//! Codex rollout jsonl: `$CODEX_HOME/sessions/**/rollout-*.jsonl`.
//!
//! A last line with no newline is still being written and is dropped before
//! UTF-8 decode, including a tail cut mid-character.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crewmux_types::RuntimeKind;
use serde_json::Value;

use crate::error::Error;
use crate::source::{TranscriptCursor, TranscriptSource, UserTurn, transcript_len};

/// `session_meta` from one rollout file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionMeta {
    /// `payload.id`. The rollout filename contains this id.
    pub id: String,
    /// `payload.cwd`.
    pub cwd: PathBuf,
    /// `payload.timestamp`, or the row timestamp.
    pub timestamp: SystemTime,
}

/// One rollout file.
#[derive(Clone, Debug)]
pub struct CodexRollout {
    path: PathBuf,
}

impl CodexRollout {
    /// Read `path` as a rollout. The file may not exist yet.
    #[must_use]
    pub fn open(path: PathBuf) -> Self {
        Self { path }
    }

    /// Path of the jsonl file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl TranscriptSource for CodexRollout {
    fn runtime(&self) -> RuntimeKind {
        RuntimeKind::Codex
    }

    fn cursor(&self) -> Result<TranscriptCursor, Error> {
        Ok(TranscriptCursor(transcript_len(&self.path)?))
    }

    fn user_turns_after(&self, after: TranscriptCursor) -> Result<Vec<UserTurn>, Error> {
        match complete_text(&self.path) {
            Ok(text) => user_turns(&self.path, &text, after.0),
            Err(Error::Incomplete { .. }) => Ok(Vec::new()),
            Err(Error::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(err) => Err(err),
        }
    }
}

/// Session row from a rollout. An unfinished file is [`Error::Incomplete`].
///
/// # Errors
///
/// Returns [`Error::Incomplete`] when the file has no complete line,
/// [`Error::Parse`] when a complete line is not UTF-8 JSON or has no session
/// row, and [`Error::Io`] when the file cannot be read.
pub fn read_session_meta(path: &Path) -> Result<SessionMeta, Error> {
    let text = complete_text(path)?;
    let mut meta = None;
    for (index, line) in text.split_inclusive('\n').enumerate() {
        let line = line.strip_suffix('\n').unwrap_or(line);
        if line.trim().is_empty() {
            continue;
        }
        let value = serde_json::from_str::<Value>(line).map_err(|err| Error::Parse {
            path: path.to_path_buf(),
            line: index.saturating_add(1),
            msg: err.to_string(),
        })?;
        if let Some(found) = session_meta(path, &value)? {
            meta = Some(found);
            break;
        }
    }
    meta.ok_or_else(|| Error::Parse {
        path: path.to_path_buf(),
        line: 1,
        msg: "session_meta missing".to_owned(),
    })
}

fn complete_text(path: &Path) -> Result<String, Error> {
    let bytes = fs::read(path)?;
    if bytes.is_empty() {
        return Err(Error::Parse {
            path: path.to_path_buf(),
            line: 1,
            msg: "empty rollout".to_owned(),
        });
    }
    let Some(kept) = complete_prefix(&bytes) else {
        return Err(Error::Incomplete {
            path: path.to_path_buf(),
        });
    };
    std::str::from_utf8(kept)
        .map(str::to_owned)
        .map_err(|err| Error::Parse {
            path: path.to_path_buf(),
            line: 1,
            msg: err.to_string(),
        })
}

fn complete_prefix(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.last() == Some(&b'\n') {
        return Some(bytes);
    }
    let index = bytes.iter().rposition(|byte| *byte == b'\n')?;
    bytes.get(..=index)
}

fn user_turns(path: &Path, text: &str, after: u64) -> Result<Vec<UserTurn>, Error> {
    let mut turns = Vec::new();
    let mut offset = 0usize;
    for (index, line) in text.split_inclusive('\n').enumerate() {
        let start = offset;
        offset = offset.saturating_add(line.len());
        let line = line.strip_suffix('\n').unwrap_or(line);
        if line.trim().is_empty() {
            continue;
        }
        let value = serde_json::from_str::<Value>(line).map_err(|err| Error::Parse {
            path: path.to_path_buf(),
            line: index.saturating_add(1),
            msg: err.to_string(),
        })?;
        let Some(text) = user_text(&value) else {
            continue;
        };
        let at = u64::try_from(start).unwrap_or(u64::MAX);
        if at >= after {
            turns.push(UserTurn {
                text,
                at: TranscriptCursor(at),
            });
        }
    }
    Ok(turns)
}

fn session_meta(path: &Path, value: &Value) -> Result<Option<SessionMeta>, Error> {
    if value.get("type").and_then(Value::as_str) != Some("session_meta") {
        return Ok(None);
    }
    let payload = value.get("payload").ok_or_else(|| Error::Parse {
        path: path.to_path_buf(),
        line: 1,
        msg: "session_meta payload missing".to_owned(),
    })?;
    let id = payload
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Parse {
            path: path.to_path_buf(),
            line: 1,
            msg: "session_meta id missing".to_owned(),
        })?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if !name.contains(id) {
        return Err(Error::Parse {
            path: path.to_path_buf(),
            line: 1,
            msg: "rollout filename does not contain the session id".to_owned(),
        });
    }
    let cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Parse {
            path: path.to_path_buf(),
            line: 1,
            msg: "session_meta cwd missing".to_owned(),
        })?;
    let stamp = payload
        .get("timestamp")
        .and_then(Value::as_str)
        .or_else(|| value.get("timestamp").and_then(Value::as_str))
        .ok_or_else(|| Error::Parse {
            path: path.to_path_buf(),
            line: 1,
            msg: "session_meta timestamp missing".to_owned(),
        })?;
    let timestamp = parse_rfc3339(stamp).ok_or_else(|| Error::Parse {
        path: path.to_path_buf(),
        line: 1,
        msg: "session_meta timestamp is not RFC3339".to_owned(),
    })?;
    Ok(Some(SessionMeta {
        id: id.to_owned(),
        cwd: PathBuf::from(cwd),
        timestamp,
    }))
}

fn user_text(value: &Value) -> Option<String> {
    let payload = value.get("payload").unwrap_or(value);
    let role = payload.get("role").and_then(Value::as_str);
    let kind = payload.get("type").and_then(Value::as_str);
    if role != Some("user") && kind != Some("user_message") {
        return None;
    }
    if let Some(text) = payload.get("text").and_then(Value::as_str) {
        return Some(text.to_owned());
    }
    if let Some(message) = payload.get("message").and_then(Value::as_str) {
        return Some(message.to_owned());
    }
    let content = payload.get("content")?.as_array()?;
    let mut combined = String::new();
    for item in content {
        if let Some(text) = item.get("text").and_then(Value::as_str) {
            combined.push_str(text);
        }
    }
    if combined.is_empty() {
        None
    } else {
        Some(combined)
    }
}

fn parse_rfc3339(text: &str) -> Option<SystemTime> {
    let text = text.trim().trim_end_matches('Z');
    let (date, time) = text.split_once('T')?;
    let mut dates = date.split('-');
    let year: i32 = dates.next()?.parse().ok()?;
    let month: u32 = dates.next()?.parse().ok()?;
    let day: u32 = dates.next()?.parse().ok()?;
    if dates.next().is_some() {
        return None;
    }
    let hms = time.split_once('.').map_or(time, |(clock, _)| clock);
    let mut clock = hms.split(':');
    let hour: u32 = clock.next()?.parse().ok()?;
    let minute: u32 = clock.next()?.parse().ok()?;
    let second: u32 = clock.next()?.parse().ok()?;
    if clock.next().is_some() || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    let secs = days
        .checked_mul(86_400)?
        .checked_add(i64::from(hour).checked_mul(3_600)?)?
        .checked_add(i64::from(minute).checked_mul(60)?)?
        .checked_add(i64::from(second))?;
    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(u64::try_from(secs).ok()?))
}

fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || day == 0 || day > 31 {
        return None;
    }
    let year = i64::from(year);
    let month = i64::from(month);
    let day = i64::from(day);
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}
