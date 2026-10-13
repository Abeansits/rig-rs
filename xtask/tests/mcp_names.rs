//! The diff projection maps every legacy MCP name onto the registered Rust tool.

use crewmux_mcp::{LEGACY_NAMES, TOOL_NAMES};
use xtask::diff::map_tool;

#[test]
fn legacy_names_map_onto_the_rust_tools() {
    let mut mapped = Vec::new();
    for name in LEGACY_NAMES {
        if let Some(rust_name) = map_tool(name) {
            mapped.push(rust_name);
        }
    }
    let mut expected: Vec<_> = TOOL_NAMES.iter().map(|name| (*name).to_owned()).collect();
    expected.sort();
    mapped.sort();
    assert_eq!(mapped, expected);
    assert_eq!(map_tool("rig_discover"), None);
    assert_eq!(map_tool("rig_bind"), None);
    assert_eq!(map_tool("rig_bundle_inspect"), None);
    assert_eq!(map_tool("agent_send").as_deref(), Some("agent_send"));
}
