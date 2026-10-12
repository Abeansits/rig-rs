//! Claude Code hook stdin, parsed into a typed event.
//!
//! Unknown event names and extra keys are kept as [`HookEvent::Unknown`].
//! A malformed document is an error; the `rig hook` command logs it and exits 0.

use std::path::PathBuf;

use serde_json::Value;

use crate::error::Error;

/// Whether compaction was requested with `/compact` or by Claude's own window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactTrigger {
    /// The operator sent `/compact`.
    Manual,
    /// Claude compacted because the context window filled.
    Auto,
}

/// How a Claude session started, from `SessionStart.source`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionSource {
    /// A fresh process.
    Startup,
    /// Compaction finished and the session continued.
    Compact,
    /// `--resume` or `/resume`.
    Resume,
    /// Any other source string, including `clear` and `fork`.
    Other(String),
}

/// One hook invocation.
#[derive(Clone, Debug, PartialEq)]
pub enum HookEvent {
    /// `SessionStart`.
    SessionStart {
        /// Claude session id.
        session_id: String,
        /// Why the session started.
        source: SessionSource,
        /// Tokens reported on resume, when Claude sent them.
        context_tokens: Option<u64>,
    },
    /// `UserPromptSubmit`.
    UserPromptSubmit {
        /// Claude session id.
        session_id: String,
    },
    /// `PreCompact`.
    PreCompact {
        /// Claude session id.
        session_id: String,
        /// Manual `/compact` or automatic compaction.
        trigger: CompactTrigger,
        /// Text after `/compact`. `null` when the operator passed none, and for `auto`.
        custom_instructions: Option<String>,
        /// Transcript path from the hook, when present.
        transcript_path: PathBuf,
        /// Working directory from the hook, when present.
        cwd: PathBuf,
    },
    /// `PostCompact`.
    PostCompact {
        /// Claude session id.
        session_id: String,
        /// Manual or automatic.
        trigger: CompactTrigger,
        /// Summary Claude wrote. Its sha256 and length are stored on the attempt.
        compact_summary: String,
    },
    /// `Stop`.
    Stop {
        /// Claude session id.
        session_id: String,
    },
    /// `SessionEnd`.
    SessionEnd {
        /// Claude session id.
        session_id: String,
        /// Why the session ended.
        reason: String,
    },
    /// An event this build does not drive. Logged, never failed.
    Unknown {
        /// `hook_event_name`, or empty when the value was not an object.
        name: String,
        /// The original JSON.
        raw: Value,
    },
}

/// Parse one hook JSON document.
///
/// # Errors
///
/// Returns [`Error::Hook`] when `text` is not JSON.
pub fn parse_hook_json(text: &str) -> Result<HookEvent, Error> {
    let value = serde_json::from_str(text).map_err(|err| Error::Hook(err.to_string()))?;
    Ok(parse_hook_value(&value))
}

/// Classify a JSON value. Extra keys are ignored. An unknown name is [`HookEvent::Unknown`].
#[must_use]
pub fn parse_hook_value(value: &Value) -> HookEvent {
    let Some(object) = value.as_object() else {
        return HookEvent::Unknown {
            name: String::new(),
            raw: value.clone(),
        };
    };
    let name = object
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or("");
    match name {
        "SessionStart" => session_start(object, value),
        "UserPromptSubmit" => with_session(object, value, |session_id| {
            HookEvent::UserPromptSubmit { session_id }
        }),
        "PreCompact" => pre_compact(object, value),
        "PostCompact" => post_compact(object, value),
        "Stop" => with_session(object, value, |session_id| HookEvent::Stop { session_id }),
        "SessionEnd" => session_end(object, value),
        other => HookEvent::Unknown {
            name: other.to_owned(),
            raw: value.clone(),
        },
    }
}

fn session_start(object: &serde_json::Map<String, Value>, raw: &Value) -> HookEvent {
    let Some(session_id) = session_id(object) else {
        return unknown(raw);
    };
    let source = match object.get("source").and_then(Value::as_str).unwrap_or("") {
        "startup" => SessionSource::Startup,
        "compact" => SessionSource::Compact,
        "resume" => SessionSource::Resume,
        other => SessionSource::Other(other.to_owned()),
    };
    let context_tokens = object.get("context_tokens").and_then(Value::as_u64);
    HookEvent::SessionStart {
        session_id,
        source,
        context_tokens,
    }
}

fn pre_compact(object: &serde_json::Map<String, Value>, raw: &Value) -> HookEvent {
    let (Some(session_id), Some(trigger)) = (session_id(object), trigger(object)) else {
        return unknown(raw);
    };
    let custom_instructions = object
        .get("custom_instructions")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let transcript_path = path_field(object, "transcript_path");
    let cwd = path_field(object, "cwd");
    HookEvent::PreCompact {
        session_id,
        trigger,
        custom_instructions,
        transcript_path,
        cwd,
    }
}

fn path_field(object: &serde_json::Map<String, Value>, key: &str) -> PathBuf {
    object
        .get(key)
        .and_then(Value::as_str)
        .map_or_else(PathBuf::new, PathBuf::from)
}

fn post_compact(object: &serde_json::Map<String, Value>, raw: &Value) -> HookEvent {
    let Some(session_id) = session_id(object) else {
        return unknown(raw);
    };
    let Some(trigger) = trigger(object) else {
        return unknown(raw);
    };
    let Some(compact_summary) = object.get("compact_summary").and_then(Value::as_str) else {
        return unknown(raw);
    };
    HookEvent::PostCompact {
        session_id,
        trigger,
        compact_summary: compact_summary.to_owned(),
    }
}

fn session_end(object: &serde_json::Map<String, Value>, raw: &Value) -> HookEvent {
    let Some(session_id) = session_id(object) else {
        return unknown(raw);
    };
    let reason = object
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("other")
        .to_owned();
    HookEvent::SessionEnd { session_id, reason }
}

fn with_session(
    object: &serde_json::Map<String, Value>,
    raw: &Value,
    build: impl FnOnce(String) -> HookEvent,
) -> HookEvent {
    session_id(object).map_or_else(|| unknown(raw), build)
}

fn session_id(object: &serde_json::Map<String, Value>) -> Option<String> {
    object
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn trigger(object: &serde_json::Map<String, Value>) -> Option<CompactTrigger> {
    match object.get("trigger").and_then(Value::as_str)? {
        "manual" => Some(CompactTrigger::Manual),
        "auto" => Some(CompactTrigger::Auto),
        _ => None,
    }
}

fn unknown(raw: &Value) -> HookEvent {
    let name = raw
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    HookEvent::Unknown {
        name,
        raw: raw.clone(),
    }
}
