//! Small JSON helpers. Object key order follows `serde_json` (sorted). Role
//! source order is recovered from the YAML text, not from this map.

use serde_json::{Map, Value};

use super::error::WorkflowParseError;

pub(crate) fn object(value: &Value) -> Option<&Map<String, Value>> {
    value.as_object()
}

pub(crate) fn is_mapping(value: &Value) -> bool {
    value.is_object()
}

pub(crate) fn js(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(true) => "true".to_owned(),
        Value::Bool(false) => "false".to_owned(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => format!("\"{}\"", escape(text)),
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(js).collect();
            format!("[{}]", parts.join(","))
        }
        Value::Object(_) => value.to_string(),
    }
}

fn escape(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out
}

pub(crate) fn integer(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|wide| i64::try_from(wide).ok()))
}

pub(crate) fn version_label(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Null => "null".to_owned(),
        Value::Array(_) | Value::Object(_) => js(value),
    }
}

pub(crate) fn reject_unknown(
    node: Option<&Value>,
    allowed: &[&str],
    path: &str,
    source: &str,
) -> Result<(), WorkflowParseError> {
    let Some(value) = node else {
        return Ok(());
    };
    let Some(map) = object(value) else {
        return Ok(());
    };
    for key in map.keys() {
        if !allowed.contains(&key.as_str()) {
            let list = allowed.join(", ");
            return Err(WorkflowParseError::new(
                "spec_unknown_key",
                format!(
                    "workflow spec at {source}: unknown key \"{key}\" at {path}. Allowed keys: [{list}]. Unknown keys were silently DROPPED before 0.4.6 (the spec looked accepted while the field did nothing); they now fail loud — remove the key or fix its spelling."
                ),
                Some(path.to_owned()),
            ));
        }
    }
    Ok(())
}

pub(crate) fn string_list(value: &Value) -> Option<Vec<String>> {
    let items = value.as_array()?;
    let mut out = Vec::new();
    for item in items {
        out.push(item.as_str()?.to_owned());
    }
    Some(out)
}
