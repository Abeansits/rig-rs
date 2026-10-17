//! Hook spool lines. A trailing partial line is kept for the next drain.

use serde_json::Value;

/// One complete spool record.
#[derive(Clone, Debug, PartialEq)]
pub struct SpoolEvent {
    /// `RIG_RUN_ID` when the hook process recorded it.
    pub run_id: Option<String>,
    /// Hook JSON.
    pub payload: Value,
}

/// Complete lines, plus the unfinished tail to leave in the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpoolText {
    /// Lines that end in a newline.
    pub complete: Vec<String>,
    /// Bytes after the last newline. Empty when the file ends in a newline.
    pub remainder: String,
}

/// Split `text` into complete lines and a partial tail.
#[must_use]
pub fn split_spool_text(text: &str) -> SpoolText {
    if text.is_empty() || text.ends_with('\n') {
        return SpoolText {
            complete: nonempty_lines(text),
            remainder: String::new(),
        };
    }
    let mut lines = text.lines().map(str::to_owned).collect::<Vec<_>>();
    let remainder = lines.pop().unwrap_or_default();
    lines.retain(|line| !line.is_empty());
    SpoolText {
        complete: lines,
        remainder,
    }
}

/// Parse one spool line. A raw hook object is accepted with no run id.
#[must_use]
pub fn parse_spool_line(line: &str) -> Option<SpoolEvent> {
    let value: Value = serde_json::from_str(line).ok()?;
    let object = value.as_object()?;
    if object.contains_key("payload") {
        let payload = object.get("payload")?.clone();
        let run_id = object
            .get("run_id")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_owned);
        return Some(SpoolEvent { run_id, payload });
    }
    if object.contains_key("hook_event_name") {
        return Some(SpoolEvent {
            run_id: None,
            payload: value,
        });
    }
    None
}

fn nonempty_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}
