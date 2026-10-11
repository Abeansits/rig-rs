//! Verdicts, projections, and unified diffs.

use serde_json::Value;

use super::types::{DiffVerdict, Observation, PeId, Project};
use crate::error::Error;

/// Compare two canonical observations.
///
/// # Errors
///
/// Returns [`Error::Stale`] when `expected` is set and the texts are equal.
pub fn judge(
    case_id: &str,
    left: &str,
    right: &str,
    expected: Option<&PeId>,
) -> Result<DiffVerdict, Error> {
    if left == right {
        return match expected {
            Some(pe) => Err(Error::Stale {
                case: case_id.to_owned(),
                pe: pe.clone(),
            }),
            None => Ok(DiffVerdict::Equal),
        };
    }
    match expected {
        Some(pe) => Ok(DiffVerdict::ExpectedDiff(pe.clone())),
        None => Ok(DiffVerdict::Unexpected(unified_diff(left, right))),
    }
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
