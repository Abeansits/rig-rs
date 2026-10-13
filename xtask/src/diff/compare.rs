//! Verdicts, projections, and unified diffs.

use serde_json::{Map, Value};

use super::types::{DiffVerdict, ObsField, Observation, PeId, Project};
use crate::error::Error;

/// Scope of one expected diff: the fields it excuses, and the Rust snapshot it still describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedScope {
    /// Exception id.
    pub pe: PeId,
    /// Fields that may differ from the TypeScript observation.
    pub covers: Vec<ObsField>,
    /// Canonical Rust observation this exception still describes.
    pub rust_snapshot: String,
}

/// Compare two canonical observations.
///
/// With no scope, equal text is [`DiffVerdict::Equal`] and any other text is
/// [`DiffVerdict::Unexpected`]. With a scope, [`DiffVerdict::ExpectedDiff`]
/// applies only when the Rust text matches `rust_snapshot` and every difference
/// from TypeScript is in [`ExpectedScope::covers`]. A difference in any other
/// field is [`DiffVerdict::Unexpected`].
///
/// # Errors
///
/// Returns [`Error::Stale`] when the two sides are equal, or when the Rust text
/// no longer matches the exception snapshot. Returns [`Error::Json`] or
/// [`Error::Message`] when a scoped observation is not canonical JSON.
pub fn judge(
    case_id: &str,
    ts: &str,
    rs: &str,
    expected: Option<&ExpectedScope>,
) -> Result<DiffVerdict, Error> {
    let Some(expected) = expected else {
        return Ok(unscoped(ts, rs));
    };
    judge_scoped(case_id, ts, rs, expected)
}

fn unscoped(ts: &str, rs: &str) -> DiffVerdict {
    if ts == rs {
        DiffVerdict::Equal
    } else {
        DiffVerdict::Unexpected(unified_diff(ts, rs))
    }
}

fn judge_scoped(
    case_id: &str,
    ts: &str,
    rs: &str,
    expected: &ExpectedScope,
) -> Result<DiffVerdict, Error> {
    let ts_obj = object_of(ts, "typescript observation")?;
    let rs_obj = object_of(rs, "rust observation")?;
    let snap_obj = object_of(&expected.rust_snapshot, "rust snapshot")?;
    if uncovered_differ(&ts_obj, &rs_obj, &expected.covers)? {
        return Ok(DiffVerdict::Unexpected(unified_diff(ts, rs)));
    }
    if !same_fields(&rs_obj, &snap_obj)? {
        return Err(stale(
            case_id,
            expected,
            "the Rust output no longer matches the exception snapshot",
        ));
    }
    if covered_equal(&ts_obj, &rs_obj, &expected.covers)? {
        return Err(stale(case_id, expected, "the outputs are equal"));
    }
    Ok(DiffVerdict::ExpectedDiff(expected.pe.clone()))
}

fn stale(case_id: &str, expected: &ExpectedScope, detail: &str) -> Error {
    Error::Stale {
        case: case_id.to_owned(),
        pe: expected.pe.clone(),
        detail: detail.to_owned(),
    }
}

fn object_of(text: &str, context: &str) -> Result<Map<String, Value>, Error> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(Error::Message(format!("{context} is not a JSON object"))),
        Err(source) => Err(Error::Json {
            context: context.to_owned(),
            source,
        }),
    }
}

fn uncovered_differ(
    ts: &Map<String, Value>,
    rs: &Map<String, Value>,
    covers: &[ObsField],
) -> Result<bool, Error> {
    for field in ObsField::ALL {
        if covers.contains(&field) {
            continue;
        }
        if field_of(ts, field)? != field_of(rs, field)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn covered_equal(
    ts: &Map<String, Value>,
    rs: &Map<String, Value>,
    covers: &[ObsField],
) -> Result<bool, Error> {
    for field in covers {
        if field_of(ts, *field)? != field_of(rs, *field)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn same_fields(left: &Map<String, Value>, right: &Map<String, Value>) -> Result<bool, Error> {
    for field in ObsField::ALL {
        if field_of(left, field)? != field_of(right, field)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn field_of(map: &Map<String, Value>, field: ObsField) -> Result<&Value, Error> {
    map.get(field.as_str()).ok_or_else(|| {
        Error::Message(format!(
            "canonical observation is missing {}",
            field.as_str()
        ))
    })
}

/// Line-oriented unified diff. Equal lines are shown with a leading space.
#[must_use]
pub fn unified_diff(left: &str, right: &str) -> String {
    let mut out = String::from("--- ts\n+++ rs\n");
    let mut left_lines = left.lines();
    let mut right_lines = right.lines();
    loop {
        match (left_lines.next(), right_lines.next()) {
            (None, None) => break,
            (Some(same_left), Some(same_right)) if same_left == same_right => {
                out.push(' ');
                out.push_str(same_left);
                out.push('\n');
            }
            (Some(old), Some(new)) => {
                push_signed(&mut out, '-', old);
                push_signed(&mut out, '+', new);
            }
            (Some(old), None) => push_signed(&mut out, '-', old),
            (None, Some(new)) => push_signed(&mut out, '+', new),
        }
    }
    out
}

fn push_signed(out: &mut String, sign: char, line: &str) {
    out.push(sign);
    out.push_str(line);
    out.push('\n');
}

/// Reduce an observation to the fields the case compares.
#[must_use]
pub fn project(obs: Observation, kind: Project) -> Observation {
    match kind {
        Project::Full => obs,
        Project::ExitCode => Observation {
            stdout: String::new(),
            stderr: String::new(),
            exit: obs.exit,
            json: None,
            files: BTreeMapEmpty::empty(),
            tmux_argv: Vec::new(),
        },
        Project::HelpNames => Observation {
            stdout: help_command_names(&obs.stdout),
            stderr: String::new(),
            exit: obs.exit,
            json: None,
            files: BTreeMapEmpty::empty(),
            tmux_argv: Vec::new(),
        },
        Project::HealthShape => Observation {
            stdout: health_shape(&obs),
            stderr: String::new(),
            exit: obs.exit,
            json: None,
            files: BTreeMapEmpty::empty(),
            tmux_argv: Vec::new(),
        },
        Project::McpToolNames => Observation {
            stdout: super::mcp_list::project_names(&obs.stdout),
            stderr: String::new(),
            exit: obs.exit,
            json: None,
            files: BTreeMapEmpty::empty(),
            tmux_argv: Vec::new(),
        },
    }
}

/// Sorted command names from a Commander-style or clap-style help page.
#[must_use]
pub fn help_command_names(help: &str) -> String {
    let mut names = Vec::new();
    let mut in_commands = false;
    for line in help.lines() {
        let trimmed = line.trim();
        if trimmed == "Commands:" || trimmed.starts_with("Commands:") {
            in_commands = true;
            continue;
        }
        if !in_commands {
            continue;
        }
        if trimmed.is_empty() {
            if !names.is_empty() {
                break;
            }
            continue;
        }
        if trimmed.starts_with("Options:") || trimmed.starts_with("Usage:") {
            break;
        }
        let Some(name) = trimmed.split_whitespace().next() else {
            continue;
        };
        if name.starts_with('-') {
            break;
        }
        names.push(name.to_owned());
    }
    names.sort();
    names.dedup();
    names.join("\n")
}

/// `status=<value>` plus the sorted top-level keys.
#[must_use]
pub(crate) fn health_shape(obs: &Observation) -> String {
    match &obs.json {
        Some(Value::Object(map)) => shape_object(map),
        Some(other) => format!("non-object:{other}"),
        None => shape_text(&obs.stdout),
    }
}

fn shape_object(map: &serde_json::Map<String, Value>) -> String {
    let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
    keys.sort_unstable();
    let status = map.get("status").and_then(Value::as_str).unwrap_or("");
    format!("status={status}\nkeys={}", keys.join(","))
}

fn shape_text(body: &str) -> String {
    match serde_json::from_str::<Value>(body) {
        Ok(Value::Object(map)) => shape_object(&map),
        Ok(other) => format!("non-object:{other}"),
        Err(_) => body.to_owned(),
    }
}

struct BTreeMapEmpty;

impl BTreeMapEmpty {
    fn empty() -> std::collections::BTreeMap<std::path::PathBuf, String> {
        std::collections::BTreeMap::new()
    }
}
