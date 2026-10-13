//! AC6–AC7: native schemas stay draft 2020-12; compat is idempotent and still validates.

#[path = "support/harness.rs"]
mod harness;

use crewmux_api::Client;
use crewmux_mcp::{Caller, RigMcp, SchemaMode, compat};
use serde_json::{Map, Value};

use harness::Session;

#[tokio::test]
async fn native_schema_is_untransformed() {
    let native = Session::start(server(SchemaMode::Native)).await;
    let compat_session = Session::start(server(SchemaMode::Compat)).await;
    let native_tools = native.list().await;
    let compat_tools = compat_session.list().await;
    assert_eq!(native_tools.len(), compat_tools.len());
    for (left, right) in native_tools.iter().zip(compat_tools.iter()) {
        let raw = Value::Object(left.input_schema.as_ref().clone());
        let listed = Value::Object(right.input_schema.as_ref().clone());
        assert_eq!(compat(raw.clone()), listed, "{}", left.name);
        assert_eq!(compat(listed.clone()), listed);
        assert!(
            compat_shape_ok(&listed),
            "{} still has a compat-forbidden shape: {listed}",
            left.name
        );
        let example = example_for(&listed);
        assert!(
            validates(&listed, &example),
            "{} {listed} {example}",
            left.name
        );
    }
}

fn server(mode: SchemaMode) -> RigMcp {
    RigMcp::new(
        Client::over_uds("/tmp/rig-mcp-schema.sock"),
        Caller::Operator,
        mode,
    )
}

fn example_for(schema: &Value) -> Value {
    let Some(props) = schema.get("properties").and_then(Value::as_object) else {
        return Value::Object(Map::new());
    };
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut object = Map::new();
    for item in required {
        let Some(name) = item.as_str() else {
            continue;
        };
        let Some(prop) = props.get(name) else {
            continue;
        };
        object.insert(name.to_owned(), example_value(prop));
    }
    Value::Object(object)
}

fn example_value(schema: &Value) -> Value {
    if let Some(branches) = schema.get("anyOf").and_then(Value::as_array)
        && let Some(first) = branches.first()
    {
        return example_value(first);
    }
    match schema.get("type").and_then(Value::as_str) {
        Some("boolean") => Value::Bool(false),
        Some("integer" | "number") => serde_json::json!(1),
        Some("array") => Value::Array(Vec::new()),
        Some("object") => example_for(schema),
        None | Some(_) => Value::String("x".to_owned()),
    }
}

fn compat_shape_ok(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            if map.contains_key("$defs") || map.contains_key("$ref") {
                return false;
            }
            if let Some(kind) = map.get("type")
                && kind.as_array().is_some()
            {
                return false;
            }
            if let Some(branches) = map.get("anyOf").and_then(Value::as_array)
                && branches.iter().any(is_null_schema)
            {
                return false;
            }
            map.values().all(compat_shape_ok)
        }
        Value::Array(items) => items.iter().all(compat_shape_ok),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => true,
    }
}

fn is_null_schema(value: &Value) -> bool {
    value.get("type").and_then(Value::as_str) == Some("null")
        || value
            .get("type")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                !items.is_empty() && items.iter().all(|item| item.as_str() == Some("null"))
            })
}

fn validates(schema: &Value, instance: &Value) -> bool {
    let Some(kind) = schema.get("type").and_then(Value::as_str) else {
        return schema.get("anyOf").is_some() || instance.is_object();
    };
    match kind {
        "object" => instance.as_object().is_some_and(|object| {
            schema
                .get("required")
                .and_then(Value::as_array)
                .is_none_or(|required| {
                    required
                        .iter()
                        .all(|item| item.as_str().is_some_and(|name| object.contains_key(name)))
                })
        }),
        "string" => instance.is_string(),
        "boolean" => instance.is_boolean(),
        "number" | "integer" => instance.is_number(),
        "array" => instance.is_array(),
        _ => true,
    }
}
