//! `RigSpec` YAML parsing and validation.
//!
//! [`yaml`] is the only module that names the YAML crate.

mod error;
mod model;
pub mod workflow;
pub mod yaml;

pub use error::SpecError;
pub use model::{LaunchBudgetSpec, RigSpec, SeatSpec};
pub use workflow::{WorkflowParseError, WorkflowSpec, parse_workflow};

use std::collections::BTreeSet;
use std::path::Path;

use crewmux_types::MAX_NAME_LEN;

/// Longest seat `cwd`, in bytes.
const MAX_CWD_LEN: usize = 4096;

/// Parse and validate a slice 1 rig spec.
///
/// # Errors
///
/// Returns [`SpecError`] for invalid YAML, unknown fields, duplicate keys,
/// an unknown runtime (including `pi`), an empty seat list, or a repeated seat name.
pub fn parse(text: &str) -> Result<RigSpec, SpecError> {
    let spec: RigSpec = yaml::parse_yaml(text)?;
    validate(&spec, text)?;
    Ok(spec)
}

fn validate(spec: &RigSpec, text: &str) -> Result<(), SpecError> {
    check_name(spec.name.as_str(), "name", text)?;
    if spec.seats.is_empty() {
        return Err(SpecError::at(
            1,
            1,
            "seats",
            "seats: must contain at least one seat",
        ));
    }
    let mut seen = BTreeSet::new();
    for (index, seat) in spec.seats.iter().enumerate() {
        let path = format!("seats[{index}].name");
        check_name(seat.name.as_str(), &path, text)?;
        if !seat_name_ok(seat.name.as_str()) {
            let (line, col) = locate_token(text, seat.name.as_str()).unwrap_or((1, 1));
            return Err(SpecError::at(
                line,
                col,
                path,
                "seat name must match [A-Za-z0-9_-]",
            ));
        }
        if let Some(model) = &seat.model {
            check_model(model, index, text)?;
        }
        if let Some(cwd) = &seat.cwd {
            check_cwd(cwd, index, text)?;
        }
        if !seen.insert(seat.name.as_str()) {
            let (line, col) = second_token(text, seat.name.as_str()).unwrap_or((1, 1));
            return Err(SpecError::at(
                line,
                col,
                path,
                format!("duplicate seat name \"{}\"", seat.name),
            ));
        }
    }
    Ok(())
}

fn seat_name_ok(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn check_name(name: &str, path: &str, text: &str) -> Result<(), SpecError> {
    if name.is_empty() || name.chars().any(char::is_control) {
        let (line, col) = locate_token(text, path).unwrap_or((1, 1));
        return Err(SpecError::at(
            line,
            col,
            path,
            "name: required non-empty string",
        ));
    }
    if name.len() > MAX_NAME_LEN {
        let (line, col) = locate_token(text, path).unwrap_or((1, 1));
        return Err(SpecError::at(
            line,
            col,
            path,
            "name must be at most 256 bytes",
        ));
    }
    Ok(())
}

fn check_model(model: &str, index: usize, text: &str) -> Result<(), SpecError> {
    if model.len() <= MAX_NAME_LEN && !model.chars().any(char::is_control) {
        return Ok(());
    }
    let (line, col) = locate_token(text, "model").unwrap_or((1, 1));
    Err(SpecError::at(
        line,
        col,
        format!("seats[{index}].model"),
        "invalid model",
    ))
}

fn check_cwd(cwd: &Path, index: usize, text: &str) -> Result<(), SpecError> {
    if cwd.as_os_str().len() <= MAX_CWD_LEN {
        return Ok(());
    }
    let (line, col) = locate_token(text, "cwd").unwrap_or((1, 1));
    Err(SpecError::at(
        line,
        col,
        format!("seats[{index}].cwd"),
        "cwd must be at most 4096 bytes",
    ))
}

fn second_token(text: &str, token: &str) -> Option<(u32, u32)> {
    let (first_line, first_col) = locate_token(text, token)?;
    let mut skipped = false;
    let mut line = 1_u32;
    let mut col = 1_u32;
    let chars: Vec<char> = text.chars().collect();
    let needle: Vec<char> = token.chars().collect();
    let mut index = 0_usize;
    while index < chars.len() {
        let here = chars.get(index..index.saturating_add(needle.len())) == Some(needle.as_slice());
        if here {
            if skipped {
                return Some((line, col));
            }
            skipped = true;
        }
        match chars.get(index) {
            Some('\n') => {
                line = line.saturating_add(1);
                col = 1;
            }
            Some(_) => col = col.saturating_add(1),
            None => break,
        }
        index = index.saturating_add(1);
    }
    let _ = (first_line, first_col);
    None
}

fn locate_token(text: &str, token: &str) -> Option<(u32, u32)> {
    let mut line = 1_u32;
    let mut col = 1_u32;
    let chars: Vec<char> = text.chars().collect();
    let needle: Vec<char> = token.chars().collect();
    if needle.is_empty() {
        return None;
    }
    let mut index = 0_usize;
    while index < chars.len() {
        if chars.get(index..index.saturating_add(needle.len())) == Some(needle.as_slice()) {
            return Some((line, col));
        }
        match chars.get(index) {
            Some('\n') => {
                line = line.saturating_add(1);
                col = 1;
            }
            Some(_) => col = col.saturating_add(1),
            None => break,
        }
        index = index.saturating_add(1);
    }
    None
}
