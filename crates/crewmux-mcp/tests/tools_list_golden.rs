//! AC1–AC2: `tools/list` matches the golden and legacy names are absent.

#[path = "support/harness.rs"]
mod harness;

use std::path::PathBuf;

use crewmux_api::Client;
use crewmux_mcp::{Caller, LEGACY_NAMES, RIG_MCP_NAME, RigMcp, SchemaMode, TOOL_NAMES};
use serde_json::Value;

use harness::Session;

#[tokio::test]
async fn tools_list_is_the_fifteen_names() {
    let session = Session::start(local_server(SchemaMode::Native)).await;
    assert_eq!(session.server_name(), RIG_MCP_NAME);
    let tools = session.list().await;
    let names: Vec<_> = tools.iter().map(|tool| tool.name.to_string()).collect();
    let mut expected: Vec<_> = TOOL_NAMES.iter().map(|name| (*name).to_owned()).collect();
    expected.sort();
    assert_eq!(names, expected);
    for legacy in LEGACY_NAMES {
        assert!(!names.iter().any(|name| name == legacy), "{legacy}");
    }
    let err = session
        .call("rig_send", serde_json::json!({}))
        .await
        .expect_err("legacy");
    assert!(err.contains("unknown tool"), "{err}");
}

#[tokio::test]
async fn tools_list_matches_golden() {
    let session = Session::start(local_server(SchemaMode::Native)).await;
    let tools = session.list().await;
    let value = harness::canonical(&serde_json::json!({
        "tools": tools.iter().map(harness::tool_value).collect::<Vec<_>>(),
    }));
    let text = serde_json::to_string_pretty(&value).expect("json");
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/tools-list.json");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        std::fs::write(&path, format!("{text}\n")).expect("write");
    }
    let saved = std::fs::read_to_string(&path).expect("golden");
    let saved: Value = serde_json::from_str(&saved).expect("parse");
    assert_eq!(harness::canonical(&saved), value);
}

#[test]
fn provider_tool_names_match() {
    assert_eq!(
        TOOL_NAMES.as_slice(),
        crewmux_provider::MCP_TOOL_NAMES.as_slice()
    );
    assert_eq!(RIG_MCP_NAME, crewmux_provider::RIG_MCP_NAME);
}

fn local_server(schema: SchemaMode) -> RigMcp {
    RigMcp::new(
        Client::over_uds("/tmp/rig-mcp-no-daemon.sock"),
        Caller::Operator,
        schema,
    )
}

#[tokio::test]
async fn stdio_reconnect_lists_fifteen() {
    let first = Session::start(local_server(SchemaMode::Native)).await;
    assert_eq!(first.list().await.len(), 15);
    drop(first);
    let second = Session::start(local_server(SchemaMode::Native)).await;
    assert_eq!(second.list().await.len(), 15);
}

#[tokio::test]
async fn bundle_inspect_is_unknown_tool() {
    let session = Session::start(local_server(SchemaMode::Native)).await;
    let err = session
        .call(
            "rig_bundle_inspect",
            serde_json::json!({"bundlePath": "/tmp/x.rigbundle"}),
        )
        .await
        .expect_err("dropped");
    assert!(err.contains("unknown tool"), "{err}");
}

#[tokio::test]
async fn schemas_have_required_fields() {
    let session = Session::start(local_server(SchemaMode::Native)).await;
    let tools = session.list().await;
    let start = find(&tools, "rig_start");
    assert!(required(start).contains(&"sourceRef".to_owned()));
    assert!(start.input_schema.contains_key("properties"));
    let props = &start.input_schema["properties"];
    assert!(props.get("plan").is_some());
    assert!(props.get("autoApprove").is_some());
    let stop = find(&tools, "rig_stop");
    assert!(required(stop).contains(&"rigId".to_owned()));
    let restore = find(&tools, "snapshot_restore");
    let names = required(restore);
    assert!(names.contains(&"rigId".to_owned()));
    assert!(names.contains(&"snapshotId".to_owned()));
}

#[tokio::test]
async fn spec_validate_tools_require_yaml() {
    let session = Session::start(local_server(SchemaMode::Native)).await;
    let tools = session.list().await;
    for name in ["spec_validate_rig", "spec_validate_agent"] {
        let tool = find(&tools, name);
        assert!(required(tool).contains(&"yaml".to_owned()), "{name}");
    }
}

#[expect(clippy::expect_used, reason = "a missing tool fails the schema test")]
fn find<'a>(tools: &'a [rmcp::model::Tool], name: &str) -> &'a rmcp::model::Tool {
    tools.iter().find(|tool| tool.name == name).expect(name)
}

fn required(tool: &rmcp::model::Tool) -> Vec<String> {
    tool.input_schema
        .get("required")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}
