// Derived from openrig packages/cli/src/mcp-server.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! ADR-0002 tool names and the legacy names they replace.

/// Server name in `initialize` and in provider registration.
pub const RIG_MCP_NAME: &str = "rig";

/// The 15 tools this server registers, in spec order.
pub const TOOL_NAMES: [&str; 15] = [
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

/// The 18 legacy tool names, sorted as `mcp-server.test.ts` lists them.
///
/// None of these are registered. `rig_discover`, `rig_bind`, and
/// `rig_bundle_inspect` were dropped. The other fifteen were renamed.
pub const LEGACY_NAMES: [&str; 18] = [
    "rig_add",
    "rig_agent_validate",
    "rig_bind",
    "rig_bundle_inspect",
    "rig_capture",
    "rig_chatroom_send",
    "rig_chatroom_watch",
    "rig_discover",
    "rig_down",
    "rig_ps",
    "rig_restore",
    "rig_rig_nodes",
    "rig_rig_validate",
    "rig_send",
    "rig_snapshot_create",
    "rig_snapshot_list",
    "rig_status",
    "rig_up",
];

/// `true` when `name` is one of [`TOOL_NAMES`].
#[must_use]
pub fn is_rig_tool(name: &str) -> bool {
    TOOL_NAMES.contains(&name)
}
