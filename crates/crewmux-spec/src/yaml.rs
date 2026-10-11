//! YAML via serde-saphyr. Callers outside this crate never name that crate.

use serde::Deserialize;
use serde_saphyr::{DuplicateKeyPolicy, Options, from_str_with_options};

use crate::error::SpecError;

/// Parse `text` as `T` with duplicate keys rejected and a document budget.
///
/// # Errors
///
/// Returns [`SpecError`] with the parser's line and column when YAML is invalid,
/// a key is repeated, or `T` rejects the value.
pub fn parse_yaml<T>(text: &str) -> Result<T, SpecError>
where
    T: for<'de> Deserialize<'de>,
{
    let mut options = Options::default();
    options.duplicate_keys = DuplicateKeyPolicy::Error;
    if let Some(budget) = options.budget.as_mut() {
        budget.max_documents = 1;
        budget.max_depth = 32;
        budget.max_anchors = 8;
        budget.max_aliases = 8;
    }
    from_str_with_options(text, options).map_err(|err| map_error(&err))
}

fn map_error(err: &serde_saphyr::Error) -> SpecError {
    let (line, col) = err.location().map_or((1, 1), |loc| {
        let line = u32::try_from(loc.line()).unwrap_or(u32::MAX);
        let col = u32::try_from(loc.column()).unwrap_or(u32::MAX);
        (line, col)
    });
    let raw = short_message(err);
    let path = path_from_message(&raw);
    let message = polish(&raw, &path);
    SpecError::at(line, col, path, message)
}

fn polish(raw: &str, path: &str) -> String {
    let trimmed = match raw.rsplit_once(" at line ") {
        Some((head, _)) if !head.is_empty() => head,
        _ => raw,
    };
    if trimmed.contains("duplicate mapping key") {
        let key = if path.is_empty() { "key" } else { path };
        return format!("duplicate key \"{key}\"");
    }
    trimmed.to_owned()
}

fn short_message(err: &serde_saphyr::Error) -> String {
    let rendered = err.without_snippet().to_string();
    let first = rendered.lines().next().unwrap_or(rendered.as_str());
    match first
        .split_once(": ")
        .and_then(|(_, rest)| rest.split_once(": "))
    {
        Some((_, reason)) if !reason.is_empty() => reason.to_owned(),
        _ => first.to_owned(),
    }
}

fn path_from_message(message: &str) -> String {
    if let Some(field) = quoted_after(message, "unknown field ") {
        return field;
    }
    if let Some(field) = quoted_after(message, "duplicate mapping key: ") {
        return field;
    }
    if let Some(field) = quoted_after(message, "unknown variant ") {
        return field;
    }
    String::new()
}

fn quoted_after(message: &str, marker: &str) -> Option<String> {
    let rest = message.split_once(marker)?.1;
    let rest = rest.trim_start_matches('`').trim_start_matches('"');
    let end = rest.find(['`', '"', ',', ' '])?;
    let token = rest.get(..end)?;
    if token.is_empty() {
        None
    } else {
        Some(token.to_owned())
    }
}
