//! ADR-0002 tool names the Codex seat may allow for one session.
//!
//! `seat_send` is the spike server's name and is not in this list. The exact
//! table is the slice 3 proposal; names match the `<object>_<action>` scheme.

/// The 15 ADR-0002 tool names, in spec order.
///
/// `crewmux-mcp::TOOL_NAMES` is canonical. A test compares the two copies.
pub const RIG_TOOL_NAMES: &[&str] = &[
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

/// `true` when `name` is one of [`RIG_TOOL_NAMES`].
#[must_use]
pub fn is_rig_tool(name: &str) -> bool {
    RIG_TOOL_NAMES.contains(&name)
}
