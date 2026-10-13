//! Per-seat MCP registration fragments. Launch writes these; setup does not.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::claude::McpServerEntry;
use crate::codex::RIG_MCP_NAME;

/// The 15 ADR-0002 tool names, in spec order.
///
/// `crewmux-mcp::TOOL_NAMES` is the canonical list. A test compares the two.
pub const MCP_TOOL_NAMES: [&str; 15] = [
    "rig_start",
    "rig_stop",
    "rig_list",
    "agent_list",
    "agent_add",
    "agent_send",
    "agent_read_screen",
    "chat_send",
    "chat_read",
    "snapshot_create",
    "snapshot_list",
    "snapshot_restore",
    "spec_validate_rig",
    "spec_validate_agent",
    "daemon_health",
];

/// Seat variables stamped into the MCP process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeatEnv {
    /// `RIG_SEAT_ID`.
    pub seat_id: String,
    /// `RIG_SEAT_TOKEN`.
    pub seat_token: String,
    /// `RIG_HOME`.
    pub rig_home: String,
}

/// `RIG_BIN`, otherwise the current executable, otherwise `rig`.
#[must_use]
pub fn rig_executable() -> PathBuf {
    if let Some(path) = std::env::var_os("RIG_BIN") {
        return PathBuf::from(path);
    }
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("rig"))
}

/// `mcp.json` document with one server, `rig`.
#[must_use]
pub fn claude_mcp_json(rig_bin: &Path, env: &SeatEnv) -> Value {
    let mut entry = Map::new();
    entry.insert(
        "command".to_owned(),
        Value::String(rig_bin.display().to_string()),
    );
    entry.insert(
        "args".to_owned(),
        Value::Array(vec![Value::String("mcp".to_owned())]),
    );
    entry.insert("env".to_owned(), Value::Object(env_object(env)));
    let mut servers = Map::new();
    servers.insert(RIG_MCP_NAME.to_owned(), Value::Object(entry));
    let mut root = Map::new();
    root.insert("mcpServers".to_owned(), Value::Object(servers));
    Value::Object(root)
}

/// `mcp__rig__<tool>` for each of [`MCP_TOOL_NAMES`].
#[must_use]
pub fn claude_allowed_tools() -> Vec<String> {
    MCP_TOOL_NAMES
        .iter()
        .map(|name| format!("mcp__{RIG_MCP_NAME}__{name}"))
        .collect()
}

/// One seat server entry. Empty env values are omitted.
#[must_use]
pub fn claude_mcp_entry(rig_bin: &Path, env: &SeatEnv) -> McpServerEntry {
    McpServerEntry {
        name: RIG_MCP_NAME.to_owned(),
        command: rig_bin.display().to_string(),
        args: vec!["mcp".to_owned()],
        env: seat_map(env),
    }
}

/// Codex `-c mcp_servers.rig.*` overrides. This does not touch `config.toml`.
#[must_use]
pub fn codex_mcp_overrides(rig_bin: &Path, env: &SeatEnv) -> Vec<String> {
    let mut out = vec![
        "-c".to_owned(),
        format!(
            "mcp_servers.{RIG_MCP_NAME}.command={}",
            toml_basic(&rig_bin.display().to_string())
        ),
        "-c".to_owned(),
        format!("mcp_servers.{RIG_MCP_NAME}.args=[\"mcp\"]"),
    ];
    for (key, value) in seat_map(env) {
        out.push("-c".to_owned());
        out.push(format!(
            "mcp_servers.{RIG_MCP_NAME}.env.{key}={}",
            toml_basic(&value)
        ));
    }
    out
}

fn env_object(env: &SeatEnv) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("RIG_SEAT_ID".to_owned(), Value::String(env.seat_id.clone()));
    map.insert(
        "RIG_SEAT_TOKEN".to_owned(),
        Value::String(env.seat_token.clone()),
    );
    map.insert("RIG_HOME".to_owned(), Value::String(env.rig_home.clone()));
    map
}

fn seat_map(env: &SeatEnv) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    push(&mut map, "RIG_HOME", &env.rig_home);
    push(&mut map, "RIG_SEAT_ID", &env.seat_id);
    push(&mut map, "RIG_SEAT_TOKEN", &env.seat_token);
    map
}

fn push(map: &mut BTreeMap<String, String>, key: &str, value: &str) {
    if !value.is_empty() {
        map.insert(key.to_owned(), value.to_owned());
    }
}

fn toml_basic(text: &str) -> String {
    if text
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '/' | '.'))
    {
        return text.to_owned();
    }
    let mut out = String::from("\"");
    for ch in text.chars() {
        if ch == '\\' || ch == '"' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}
