// Derived from openrig packages/cli/src/mcp-server.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Map a daemon HTTP body onto an MCP tool result.
//!
//! HTTP status at least 400, a truthy in-body `error`, a non-empty `errors`
//! array, or a tool-specific structural check all set `isError`.

use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::Value;

/// Daemon body as a text content block, with `isError` when the body failed.
#[must_use]
pub(crate) fn map_body(
    status: u16,
    body: &str,
    structural: impl Fn(&Value) -> bool,
) -> CallToolResult {
    let failed = status >= 400 || body_failed(body, structural);
    text_result(body, failed)
}

/// A JSON object the tool built locally, before any daemon call.
#[must_use]
pub(crate) fn local_error(value: &Value) -> CallToolResult {
    text_result(&value.to_string(), true)
}

/// A JSON object the tool built locally that is not an error.
#[must_use]
pub(crate) fn local_ok(value: &Value) -> CallToolResult {
    text_result(&value.to_string(), false)
}

/// Structured daemon-down payload. This is a tool error, not a protocol error.
#[must_use]
pub(crate) fn daemon_unavailable() -> CallToolResult {
    local_error(&serde_json::json!({"error": "daemon_unavailable"}))
}

fn text_result(text: &str, failed: bool) -> CallToolResult {
    let content = vec![ContentBlock::text(text.to_owned())];
    if failed {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    }
}

fn body_failed(body: &str, structural: impl Fn(&Value) -> bool) -> bool {
    let Ok(data) = serde_json::from_str::<Value>(body) else {
        return false;
    };
    truthy(data.get("error"))
        || data
            .get("errors")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty())
        || structural(&data)
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null | Value::Bool(false)) => false,
        Some(Value::Bool(true) | Value::Object(_)) => true,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|value| value != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
    }
}

/// `agent_add`: a node status other than `launched` is a structural failure.
#[must_use]
pub(crate) fn unlaunched(data: &Value) -> bool {
    data.get("result")
        .and_then(|result| result.get("node"))
        .and_then(|node| node.get("status"))
        .and_then(Value::as_str)
        .is_some_and(|status| status != "launched")
}
