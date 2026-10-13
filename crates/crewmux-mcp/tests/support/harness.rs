//! In-process MCP client over a tokio duplex pipe.
#![expect(
    dead_code,
    reason = "each integration test includes this helper and uses only the functions it needs"
)]
#![expect(clippy::expect_used, reason = "a failed MCP handshake fails the test")]

use crewmux_api::Client;
use crewmux_mcp::{Caller, RigMcp, SchemaMode, caller_from_pair};
use rmcp::model::{CallToolRequestParams, CallToolResult, Tool};
use rmcp::{ServiceExt, service::RunningService};
use serde_json::{Map, Value};

pub(crate) struct Session {
    client: RunningService<rmcp::RoleClient, ()>,
    server: tokio::task::JoinHandle<()>,
}

impl Session {
    pub(crate) async fn start(server: RigMcp) -> Self {
        let (server_io, client_io) = tokio::io::duplex(64 * 1024);
        let server = tokio::spawn(async move {
            let running = server.serve(server_io).await.expect("serve");
            drop(running.waiting().await);
        });
        let client = ().serve(client_io).await.expect("client");
        Self { client, server }
    }

    pub(crate) async fn list(&self) -> Vec<Tool> {
        self.client.list_all_tools().await.expect("tools")
    }

    pub(crate) async fn call(&self, name: &str, args: Value) -> Result<CallToolResult, String> {
        let mut params = CallToolRequestParams::new(name.to_owned());
        params.arguments = args.as_object().cloned();
        self.client
            .call_tool(params)
            .await
            .map_err(|err| err.to_string())
    }

    pub(crate) fn server_name(&self) -> String {
        self.client
            .peer_info()
            .and_then(|info| info.server_info.as_ref().map(|server| server.name.clone()))
            .unwrap_or_default()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.server.abort();
    }
}

pub(crate) fn operator(path: &std::path::Path) -> RigMcp {
    RigMcp::new(Client::over_uds(path), Caller::Operator, SchemaMode::Native)
}

pub(crate) fn seat(path: &std::path::Path, id: &str, token: &str) -> RigMcp {
    let caller = caller_from_pair(Some(id), Some(token)).expect("seat");
    RigMcp::new(Client::over_uds(path), caller, SchemaMode::Native)
}

pub(crate) fn text_of(result: &CallToolResult) -> String {
    result
        .content
        .first()
        .and_then(|block| block.as_text())
        .map(|text| text.text.clone())
        .unwrap_or_default()
}

pub(crate) fn json_of(result: &CallToolResult) -> Value {
    serde_json::from_str(&text_of(result)).unwrap_or(Value::Null)
}

pub(crate) fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().cloned().collect();
            keys.sort();
            let mut out = Map::new();
            for key in keys {
                if let Some(child) = map.get(&key) {
                    out.insert(key, canonical(child));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        other @ (Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)) => {
            other.clone()
        }
    }
}

pub(crate) fn tool_value(tool: &Tool) -> Value {
    canonical(&serde_json::json!({
        "name": tool.name.as_ref(),
        "description": tool.description.as_deref().unwrap_or(""),
        "inputSchema": Value::Object(tool.input_schema.as_ref().clone()),
    }))
}
