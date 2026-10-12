//! AC5: a down daemon is a structured error and is not started.

#[path = "support/harness.rs"]
mod harness;

use crewmux_api::Client;
use crewmux_mcp::{Caller, RigMcp, SchemaMode};

use harness::Session;

#[tokio::test]
async fn daemon_down_is_unavailable() {
    let session = Session::start(down_server()).await;
    for name in ["rig_list", "agent_list", "snapshot_list", "agent_send"] {
        let args = match name {
            "agent_send" => serde_json::json!({"session": "dev", "text": "hi"}),
            "agent_list" | "snapshot_list" => serde_json::json!({"rigId": "rig-1"}),
            _ => serde_json::json!({}),
        };
        let result = session.call(name, args).await.expect(name);
        assert_eq!(result.is_error, Some(true), "{name}");
        assert_eq!(
            harness::json_of(&result)["error"],
            "daemon_unavailable",
            "{name}"
        );
    }
}

#[tokio::test]
async fn daemon_health_reports_not_running() {
    let session = Session::start(down_server()).await;
    let result = session
        .call("daemon_health", serde_json::json!({}))
        .await
        .expect("health");
    assert_eq!(result.is_error, Some(false));
    assert_eq!(harness::json_of(&result)["running"], false);
}

#[tokio::test]
async fn spec_validate_ignores_down_daemon() {
    let session = Session::start(down_server()).await;
    let yaml = "name: demo\nseats:\n  - name: dev\n    runtime: terminal\n";
    let result = session
        .call("spec_validate_rig", serde_json::json!({"yaml": yaml}))
        .await
        .expect("validate");
    assert_eq!(
        result.is_error,
        Some(false),
        "{}",
        harness::text_of(&result)
    );
    assert_eq!(harness::json_of(&result)["valid"], true);
}

fn down_server() -> RigMcp {
    RigMcp::new(
        Client::over_uds("/tmp/rig-mcp-absent.sock"),
        Caller::Operator,
        SchemaMode::Native,
    )
}
