//! Schema fallback for a client that rejects draft 2020-12.
//!
//! Native mode is the schemars document unchanged. Compat mode inlines `$ref`,
//! rewrites `type: [T, "null"]` to `T` and drops it from `required`, and
//! collapses `anyOf` branches that are null.

use serde_json::{Map, Value};

/// Whether `tools/list` rewrites input schemas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchemaMode {
    /// schemars draft 2020-12, unchanged.
    Native,
    /// [`compat`] applied to each input schema.
    Compat,
}

impl SchemaMode {
    /// `RIG_MCP_SCHEMA_COMPAT=1` selects compat. Anything else is native.
    #[must_use]
    pub fn from_env() -> Self {
        match std::env::var("RIG_MCP_SCHEMA_COMPAT") {
            Ok(value) if value == "1" => Self::Compat,
            _ => Self::Native,
        }
    }
}

/// Inline `$defs` / `$ref`, drop null from type arrays, and collapse null `anyOf`.
///
/// The result has no `$defs`, no `$ref`, no `anyOf` that includes null, and no
/// `type` array. A second call returns the same document.
#[must_use]
#[expect(
    clippy::needless_pass_by_value,
    reason = "callers hand over the schema document"
)]
pub fn compat(schema: Value) -> Value {
    let defs = gather_defs(&schema);
    let walked = walk(&schema, &defs, &mut Vec::new());
    strip_meta(walked.value)
}

struct Walked {
    value: Value,
    nullable: bool,
}

fn walk(node: &Value, defs: &Map<String, Value>, stack: &mut Vec<String>) -> Walked {
    let Value::Object(map) = node else {
        return Walked {
            value: node.clone(),
            nullable: false,
        };
    };
    if let Some(name) = ref_name(map) {
        return inline_ref(name, defs, stack);
    }
    let mut out = Map::new();
    let mut nullable = false;
    let mut dropped_props = Vec::new();
    for (key, value) in map {
        if key == "$defs" || key == "definitions" || key == "$ref" {
            continue;
        }
        if key == "properties" {
            let (props, dropped) = walk_properties(value, defs, stack);
            out.insert(key.clone(), props);
            dropped_props = dropped;
            continue;
        }
        if key == "required" {
            out.insert(key.clone(), value.clone());
            continue;
        }
        let child = walk(value, defs, stack);
        out.insert(key.clone(), child.value);
    }
    nullable |= rewrite_type_array(&mut out);
    nullable |= collapse_anyof(&mut out, defs, stack);
    if !dropped_props.is_empty() {
        remember_dropped(&mut out, &dropped_props);
    }
    Walked {
        value: Value::Object(out),
        nullable,
    }
}

fn walk_properties(
    value: &Value,
    defs: &Map<String, Value>,
    stack: &mut Vec<String>,
) -> (Value, Vec<String>) {
    let Some(map) = value.as_object() else {
        return (walk(value, defs, stack).value, Vec::new());
    };
    let mut props = Map::new();
    let mut dropped = Vec::new();
    for (key, prop) in map {
        let child = walk(prop, defs, stack);
        if child.nullable {
            dropped.push(key.clone());
        }
        props.insert(key.clone(), child.value);
    }
    (Value::Object(props), dropped)
}

fn remember_dropped(out: &mut Map<String, Value>, dropped: &[String]) {
    let Some(required) = out.get("required").and_then(Value::as_array).cloned() else {
        return;
    };
    let kept: Vec<Value> = required
        .into_iter()
        .filter(|item| {
            item.as_str()
                .is_none_or(|name| !dropped.iter().any(|drop| drop == name))
        })
        .collect();
    out.insert("required".to_owned(), Value::Array(kept));
}

fn inline_ref(name: &str, defs: &Map<String, Value>, stack: &mut Vec<String>) -> Walked {
    if stack.iter().any(|seen| seen == name) {
        return Walked {
            value: Value::Object(Map::new()),
            nullable: false,
        };
    }
    let Some(body) = defs.get(name) else {
        return Walked {
            value: Value::Object(Map::new()),
            nullable: false,
        };
    };
    stack.push(name.to_owned());
    let walked = walk(body, defs, stack);
    stack.pop();
    walked
}

fn ref_name(map: &Map<String, Value>) -> Option<&str> {
    let text = map.get("$ref")?.as_str()?;
    text.strip_prefix("#/$defs/")
        .or_else(|| text.strip_prefix("#/definitions/"))
}

fn rewrite_type_array(obj: &mut Map<String, Value>) -> bool {
    let Some(types) = obj.get("type").and_then(Value::as_array).cloned() else {
        return false;
    };
    let mut rest = Vec::new();
    let mut nullable = false;
    for item in types {
        if item.as_str() == Some("null") {
            nullable = true;
        } else {
            rest.push(item);
        }
    }
    obj.remove("type");
    match rest.len() {
        0 => {
            obj.insert("type".to_owned(), Value::String("null".to_owned()));
        }
        1 => {
            if let Some(one) = rest.into_iter().next() {
                obj.insert("type".to_owned(), one);
            }
        }
        _ => {
            let branches = rest
                .into_iter()
                .map(|item| Value::Object(Map::from_iter([("type".to_owned(), item)])))
                .collect();
            obj.insert("anyOf".to_owned(), Value::Array(branches));
        }
    }
    nullable
}

fn collapse_anyof(
    obj: &mut Map<String, Value>,
    defs: &Map<String, Value>,
    stack: &mut Vec<String>,
) -> bool {
    let Some(branches) = obj.get("anyOf").and_then(Value::as_array).cloned() else {
        return false;
    };
    let mut rest = Vec::new();
    let mut nullable = false;
    for branch in branches {
        let walked = walk(&branch, defs, stack);
        if walked.nullable || is_null_schema(&walked.value) {
            nullable = true;
        } else {
            rest.push(walked.value);
        }
    }
    if !nullable {
        obj.insert("anyOf".to_owned(), Value::Array(rest));
        return false;
    }
    obj.remove("anyOf");
    match rest.len() {
        0 => {
            obj.insert("type".to_owned(), Value::String("null".to_owned()));
        }
        1 => {
            if let Some(one) = rest.into_iter().next() {
                merge_schema(obj, one);
            }
        }
        _ => {
            obj.insert("anyOf".to_owned(), Value::Array(rest));
        }
    }
    true
}

fn merge_schema(obj: &mut Map<String, Value>, branch: Value) {
    let Value::Object(map) = branch else {
        return;
    };
    for (key, value) in map {
        obj.entry(key).or_insert(value);
    }
}

fn is_null_schema(value: &Value) -> bool {
    value.get("type").and_then(Value::as_str) == Some("null")
        || value
            .get("type")
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().all(|item| item.as_str() == Some("null")))
}

fn gather_defs(schema: &Value) -> Map<String, Value> {
    let mut defs = Map::new();
    collect_defs(schema, &mut defs);
    defs
}

fn collect_defs(node: &Value, defs: &mut Map<String, Value>) {
    let Value::Object(map) = node else {
        return;
    };
    for key in ["$defs", "definitions"] {
        if let Some(table) = map.get(key).and_then(Value::as_object) {
            for (name, body) in table {
                defs.insert(name.clone(), body.clone());
                collect_defs(body, defs);
            }
        }
    }
    for (key, value) in map {
        if key != "$defs" && key != "definitions" {
            collect_defs(value, defs);
        }
    }
}

fn strip_meta(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, child) in map {
                if key == "$defs" || key == "definitions" || key == "$ref" {
                    continue;
                }
                out.insert(key, strip_meta(child));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(strip_meta).collect()),
        other @ (Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)) => other,
    }
}

#[cfg(test)]
mod tests {
    use super::{SchemaMode, compat};
    use serde_json::json;

    #[test]
    fn compat_inlines_and_drops_null() {
        let schema = json!({
            "$defs": { "Name": { "type": "string" } },
            "type": "object",
            "properties": {
                "name": { "$ref": "#/$defs/Name" },
                "note": { "type": ["string", "null"] },
                "flag": { "anyOf": [ { "type": "boolean" }, { "type": "null" } ] }
            },
            "required": ["name", "note", "flag"]
        });
        let once = compat(schema);
        let twice = compat(once.clone());
        assert_eq!(once, twice);
        let text = once.to_string();
        assert!(!text.contains("$defs"));
        assert!(!text.contains("$ref"));
        assert!(!text.contains("\"null\""));
        assert_eq!(once["properties"]["name"]["type"], "string");
        assert_eq!(once["properties"]["note"]["type"], "string");
        assert_eq!(once["properties"]["flag"]["type"], "boolean");
        let required = once["required"].as_array().expect("required");
        assert_eq!(required.len(), 1);
        assert_eq!(required.first().and_then(|v| v.as_str()), Some("name"));
    }

    #[test]
    fn non_null_type_array_becomes_anyof() {
        let schema = json!({ "type": ["string", "number"] });
        let out = compat(schema);
        assert!(out.get("type").is_none());
        assert_eq!(out["anyOf"][0]["type"], "string");
        assert_eq!(out["anyOf"][1]["type"], "number");
    }

    #[test]
    fn unset_compat_env_is_native() {
        assert_eq!(SchemaMode::from_env(), SchemaMode::Native);
    }
}
