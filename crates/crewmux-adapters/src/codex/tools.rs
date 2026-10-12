//! ADR-0002 tool names the Codex seat may allow for one session.
//!
//! `seat_send` is the spike server's name and is not in this list. The exact
//! table is the slice 3 proposal; names match the `<object>_<action>` scheme.

/// The 15 in-scope tool names.
pub const RIG_TOOL_NAMES: &[&str] = &[
    "rig_list",
    "rig_start",
    "rig_stop",
    "agent_list",
    "agent_send",
    "agent_read",
    "agent_add",
    "chat_send",
    "chat_read",
    "snapshot_create",
    "snapshot_restore",
    "snapshot_list",
    "spec_validate",
    "daemon_health",
    "daemon_list",
];

/// `true` when `name` is one of [`RIG_TOOL_NAMES`].
#[must_use]
pub fn is_rig_tool(name: &str) -> bool {
    RIG_TOOL_NAMES.contains(&name)
}
