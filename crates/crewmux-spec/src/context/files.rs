// Derived from openrig packages/daemon/src/domain/context-packs/manifest-parser.ts
// (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Manifest `files:` entries.

use serde_json::Value;

use super::error::ManifestError;
use super::types::{FILE_SUFFIXES, PackFile};

pub(super) fn parse_files(
    raw: Option<&Value>,
    source_path: &str,
) -> Result<Vec<PackFile>, ManifestError> {
    let Some(raw) = raw else {
        return Err(invalid(format!(
            "manifest at {source_path} must declare 'files: [...]' (got: undefined)"
        )));
    };
    let Some(entries) = raw.as_array() else {
        return Err(invalid(format!(
            "manifest at {source_path} must declare 'files: [...]' (got: {})",
            js_typeof(raw)
        )));
    };
    let mut files = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        files.push(one_file(entry, source_path, index)?);
    }
    Ok(files)
}

fn one_file(entry: &Value, source_path: &str, index: usize) -> Result<PackFile, ManifestError> {
    let Some(object) = entry.as_object() else {
        return Err(invalid(format!(
            "manifest at {source_path} has malformed entry at files[{index}] (must be an object with 'path' + 'role')"
        )));
    };
    let path = required_string(object.get("path"), source_path, index, "path")?;
    if path.contains("..") || path.starts_with('/') {
        return Err(invalid(format!(
            "manifest at {source_path} files[{index}].path '{path}' must be a relative path inside the pack (no '..' segments, no leading '/')"
        )));
    }
    let role = required_string(object.get("role"), source_path, index, "role")?;
    if !FILE_SUFFIXES.iter().any(|suffix| path.ends_with(suffix)) {
        return Err(invalid(format!(
            "manifest at {source_path} files[{index}].path '{path}' has an unsupported suffix; allowed: {}",
            FILE_SUFFIXES.join(", ")
        )));
    }
    let summary = object
        .get("summary")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(PackFile {
        path,
        role,
        summary,
    })
}

fn required_string(
    value: Option<&Value>,
    source_path: &str,
    index: usize,
    field: &str,
) -> Result<String, ManifestError> {
    match value.and_then(Value::as_str) {
        Some(text) if !text.is_empty() => Ok(text.to_owned()),
        _ => Err(invalid(format!(
            "manifest at {source_path} files[{index}] missing '{field}' (string)"
        ))),
    }
}

fn js_typeof(value: &Value) -> &'static str {
    match value {
        Value::Null | Value::Array(_) | Value::Object(_) => "object",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
    }
}

fn invalid(message: String) -> ManifestError {
    ManifestError::new(super::error::ManifestCode::Invalid, message)
}
