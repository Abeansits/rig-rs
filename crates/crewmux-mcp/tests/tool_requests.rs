//! AC4: each tool's daemon request list. Chat resolves a name, then calls once more.

#[path = "support/fake_daemon.rs"]
mod fake;
#[path = "support/harness.rs"]
mod harness;

use crewmux_mcp::caller_from_pair;
use serde_json::{Value, json};

use fake::{FakeDaemon, Hit, Scripted};
use harness::Session;

const SEAT: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";

#[test]
fn partial_seat_env_is_rejected() {
    assert!(caller_from_pair(Some(SEAT), None).is_err());
    assert!(caller_from_pair(None, Some("token")).is_err());
    assert!(caller_from_pair(None, None).is_ok());
}

#[tokio::test]
async fn agent_add_is_not_implemented() {
    let (hits, result) = call(
        vec![script(201, r#"{"ok":true}"#)],
        "agent_add",
        json!({
            "rigId": "rig/1",
            "podNamespace": "infra",
            "member": {"id": "server2"},
            "edges": [{"from": "server2", "to": "server", "kind": "delegates_to"}]
        }),
        false,
    )
    .await;
    assert!(hits.is_empty());
    assert_eq!(result.is_error, Some(true));
    assert_eq!(harness::json_of(&result)["error"], "not_implemented");
    assert_eq!(harness::json_of(&result)["feature"], "agent_add");
}

#[tokio::test]
async fn http_error_is_tool_error() {
    let (_, result) = call(
        vec![script(404, r#"{"error":"Rig not found"}"#)],
        "rig_stop",
        json!({"rigId":"missing"}),
        false,
    )
    .await;
    assert_eq!(result.is_error, Some(true));
    assert_eq!(harness::json_of(&result)["error"], "Rig not found");
}

#[tokio::test]
async fn in_body_errors_are_tool_errors() {
    let (_, result) = call(
        vec![script(200, r#"{"errors":["Kill failed"]}"#)],
        "rig_stop",
        json!({"rigId":"rig-1"}),
        false,
    )
    .await;
    assert_eq!(result.is_error, Some(true));
}

#[tokio::test]
async fn rig_stop_posts_down() {
    let (hits, _) = call(
        vec![script(200, r#"{"stopped":true}"#)],
        "rig_stop",
        json!({"rigId":"rig-1","delete":false,"force":false,"snapshot":false}),
        false,
    )
    .await;
    assert_eq!(hits[0].method, "POST");
    assert_eq!(hits[0].path, "/api/rigs/down");
    assert_eq!(body(&hits[0])["rig"], "rig-1");
    assert!(body(&hits[0]).get("rigId").is_none());
    assert!(body(&hits[0]).get("force").is_none());
}

#[tokio::test]
async fn rig_list_gets_summary() {
    let (hits, result) = call(
        vec![script(200, r#"[{"id":"r","name":"demo"}]"#)],
        "rig_list",
        json!({}),
        false,
    )
    .await;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].method, "GET");
    assert_eq!(hits[0].path, "/api/rigs/summary");
    assert_eq!(harness::json_of(&result)[0]["name"], "demo");
}

#[tokio::test]
async fn agent_list_filters_by_rig() {
    let (hits, result) = call(
        vec![
            script(200, r#"[{"id":"rig-1","name":"demo"}]"#),
            script(
                200,
                r#"{"rigs":[{"name":"demo","seats":[{"name":"dev"}]},{"name":"other","seats":[]}]}"#,
            ),
        ],
        "agent_list",
        json!({"rigId":"rig-1"}),
        false,
    )
    .await;
    assert_eq!(hits[0].path, "/api/rigs/summary");
    assert_eq!(hits[1].path, "/api/seats");
    assert_eq!(harness::json_of(&result)[0]["name"], "dev");
}

#[tokio::test]
async fn agent_send_returns_daemon_json() {
    let (hits, result) = call(
        vec![script(200, r#"{"ok":true,"delivered":1}"#)],
        "agent_send",
        json!({"session":"dev-impl@my-rig","text":"hello","verify":true}),
        true,
    )
    .await;
    assert_eq!(result.is_error, Some(false));
    assert_eq!(harness::json_of(&result)["delivered"], 1);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path, "/api/send");
    let sent = body(&hits[0]);
    assert_eq!(sent["to"], "dev-impl@my-rig");
    assert_eq!(sent["body"], "hello");
    assert_eq!(sent["verify"], true);
    assert_eq!(sent["from"], SEAT);
    assert!(sent.get("dangerously_interact").is_none());
    assert!(hits[0].headers.contains("X-Rig-Seat-Id"));
    assert!(hits[0].headers.contains(SEAT));
    assert!(hits[0].headers.contains("X-Rig-Seat-Token"));
}

#[tokio::test]
async fn agent_send_wait_is_refused() {
    let (hits, result) = call(
        vec![script(200, r#"{"ok":true}"#)],
        "agent_send",
        json!({"session":"dev","text":"hello","waitForIdleSeconds":30}),
        false,
    )
    .await;
    assert!(hits.is_empty());
    assert_eq!(result.is_error, Some(true));
    assert_eq!(harness::json_of(&result)["feature"], "waitForIdleSeconds");
}

#[tokio::test]
async fn agent_send_rejects_force_with_wait() {
    let daemon = FakeDaemon::start(vec![script(200, r#"{"ok":true}"#)]);
    let session = Session::start(harness::seat(daemon.path(), SEAT, "tok")).await;
    let result = session
        .call(
            "agent_send",
            json!({
                "session": "dev",
                "text": "hello",
                "waitForIdleSeconds": 30,
                "force": true
            }),
        )
        .await
        .expect("call");
    assert_eq!(result.is_error, Some(true));
    assert!(harness::text_of(&result).contains("waitForIdleSeconds cannot be combined with force"));
    assert!(daemon.hits().is_empty());
}

#[tokio::test]
async fn agent_read_screen_returns_capture() {
    let (hits, result) = call(
        vec![script(200, r#"{"text":"ready"}"#)],
        "agent_read_screen",
        json!({"session":"dev","lines":20}),
        false,
    )
    .await;
    assert_eq!(hits[0].path, "/api/seats/capture");
    assert_eq!(body(&hits[0])["seat"], "dev");
    assert_eq!(body(&hits[0])["lines"], 20);
    assert_eq!(harness::json_of(&result)["text"], "ready");
}

#[tokio::test]
async fn chat_send_returns_daemon_json() {
    let (hits, result) = call(
        vec![
            script(200, r#"[{"id":"r1","name":"demo"}]"#),
            script(201, r#"{"id":"m1","body":"hi"}"#),
        ],
        "chat_send",
        json!({"rigName":"demo","body":"hi"}),
        true,
    )
    .await;
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].path, "/api/rigs/summary");
    assert_eq!(hits[1].path, "/api/rigs/r1/chat/send");
    assert_eq!(body(&hits[1])["body"], "hi");
    assert!(hits[1].headers.contains("X-Rig-Seat-Token"));
    assert_eq!(result.is_error, Some(false));
}

#[tokio::test]
async fn chat_read_returns_history() {
    let (hits, result) = call(
        vec![
            script(200, r#"[{"id":"r1","name":"demo"}]"#),
            script(200, r#"{"messages":[{"body":"hi"}]}"#),
        ],
        "chat_read",
        json!({"rigName":"demo"}),
        false,
    )
    .await;
    assert_eq!(hits[1].path, "/api/rigs/r1/chat/history?limit=20");
    assert_eq!(harness::json_of(&result)["messages"][0]["body"], "hi");
}

#[tokio::test]
async fn chat_send_ambiguous_rig() {
    let (hits, result) = call(
        vec![script(
            200,
            r#"[{"id":"a","name":"demo"},{"id":"b","name":"demo"}]"#,
        )],
        "chat_send",
        json!({"rigName":"demo","body":"hi"}),
        false,
    )
    .await;
    assert_eq!(hits.len(), 1);
    assert_eq!(result.is_error, Some(true));
    let error = harness::json_of(&result)["error"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    assert!(error.contains("ambiguous"), "{error}");
    assert!(error.contains('—'), "{error}");
}

#[tokio::test]
async fn chat_send_missing_rig() {
    let (_, result) = call(
        vec![script(200, r"[]")],
        "chat_send",
        json!({"rigName":"missing","body":"hi"}),
        false,
    )
    .await;
    assert_eq!(
        harness::json_of(&result)["error"],
        "Rig 'missing' not found"
    );
}

#[tokio::test]
async fn rig_start_posts_spec_yaml() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("rig.yaml");
    let yaml = "name: demo\nseats:\n  - name: dev\n    runtime: terminal\n";
    std::fs::write(&path, yaml).expect("spec");
    let (hits, result) = call(
        vec![script(200, r#"{"name":"demo"}"#)],
        "rig_start",
        json!({"sourceRef": path.display().to_string(), "plan": false}),
        false,
    )
    .await;
    assert_eq!(hits[0].method, "POST");
    assert_eq!(hits[0].path, "/api/rigs/up");
    assert_eq!(hits[0].body, yaml);
    assert_eq!(result.is_error, Some(false));
}

#[tokio::test]
async fn snapshot_create_posts_body() {
    let (hits, _) = call(
        vec![script(200, r#"{"id":"snap-1"}"#)],
        "snapshot_create",
        json!({"rigId":"rig/1"}),
        false,
    )
    .await;
    assert_eq!(hits[0].path, "/api/snapshot/create");
    assert_eq!(body(&hits[0])["rig"], "rig/1");
}

#[tokio::test]
async fn snapshot_restore_posts_ids() {
    let (hits, _) = call(
        vec![script(200, r#"{"restored":true}"#)],
        "snapshot_restore",
        json!({"rigId":"rig-1","snapshotId":"snap-1"}),
        false,
    )
    .await;
    assert_eq!(hits[0].path, "/api/restore");
    assert_eq!(body(&hits[0])["snapshot"], "snap-1");
}

#[tokio::test]
async fn spec_validate_rig_is_local() {
    let daemon = FakeDaemon::start(vec![]);
    let session = Session::start(harness::operator(daemon.path())).await;
    let result = session
        .call("spec_validate_agent", json!({"yaml": "name: my-agent\n"}))
        .await
        .expect("agent");
    assert_eq!(result.is_error, Some(false));
    assert_eq!(harness::json_of(&result)["valid"], true);
    let bad = session
        .call("spec_validate_rig", json!({"yaml": "name: [\n"}))
        .await
        .expect("rig");
    assert_eq!(bad.is_error, Some(true));
    assert!(daemon.hits().is_empty());
}

#[tokio::test]
async fn daemon_health_returns_up_body() {
    let (hits, result) = call(
        vec![script(200, r#"{"status":"ok","pid":7}"#)],
        "daemon_health",
        json!({}),
        false,
    )
    .await;
    assert_eq!(hits[0].path, "/healthz");
    assert_eq!(harness::json_of(&result)["pid"], 7);
    assert!(harness::json_of(&result).get("version").is_none());
    assert_eq!(result.is_error, Some(false));
}

#[tokio::test]
async fn rig_stop_snapshots_then_stops() {
    let (hits, result) = call(
        vec![
            script(200, r#"{"id":"snap-1"}"#),
            script(200, r#"{"stopped":true}"#),
        ],
        "rig_stop",
        json!({"rigId":"demo","snapshot":true}),
        false,
    )
    .await;
    assert_eq!(result.is_error, Some(false));
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].path, "/api/snapshot/create");
    assert_eq!(body(&hits[0])["rig"], "demo");
    assert_eq!(hits[1].path, "/api/rigs/down");
    assert_eq!(body(&hits[1])["rig"], "demo");
    let parsed = harness::json_of(&result);
    assert_eq!(parsed["snapshot"]["id"], "snap-1");
    assert_eq!(parsed["down"]["stopped"], true);
}

#[tokio::test]
async fn rig_stop_does_not_stop_when_snapshot_fails() {
    let (hits, result) = call(
        vec![script(500, r#"{"error":"snapshot failed"}"#)],
        "rig_stop",
        json!({"rigId":"demo","snapshot":true}),
        false,
    )
    .await;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path, "/api/snapshot/create");
    assert_eq!(result.is_error, Some(true));
}

#[tokio::test]
async fn rig_stop_refuses_delete_and_force() {
    for (flag, feature) in [("delete", "delete"), ("force", "force")] {
        let (hits, result) = call(
            vec![script(200, r#"{"stopped":true}"#)],
            "rig_stop",
            json!({"rigId":"demo", flag: true, "snapshot": true}),
            false,
        )
        .await;
        assert!(hits.is_empty(), "{flag}");
        assert_eq!(harness::json_of(&result)["feature"], feature, "{flag}");
    }
}

#[tokio::test]
async fn rig_start_refuses_flags() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("rig.yaml");
    std::fs::write(
        &path,
        "name: demo\nseats:\n  - name: dev\n    runtime: terminal\n",
    )
    .expect("spec");
    let source = path.display().to_string();
    for (extra, feature) in [
        (json!({"plan": true}), "plan"),
        (json!({"autoApprove": true}), "autoApprove"),
        (json!({"targetRoot": "/tmp/elsewhere"}), "targetRoot"),
    ] {
        let mut args = json!({"sourceRef": source});
        let Some(map) = args.as_object_mut() else {
            panic!("args");
        };
        let Some(extra) = extra.as_object() else {
            panic!("extra");
        };
        for (key, value) in extra {
            map.insert(key.clone(), value.clone());
        }
        let (hits, result) = call(
            vec![script(200, r#"{"name":"demo"}"#)],
            "rig_start",
            args,
            false,
        )
        .await;
        assert!(hits.is_empty(), "{feature}");
        assert_eq!(harness::json_of(&result)["feature"], feature, "{feature}");
    }
}

#[tokio::test]
async fn rig_start_hides_non_spec_keys() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("not-a-spec.env");
    let secret = "sk-live-SUPERSECRET-key";
    std::fs::write(&path, format!("{secret}: hunter2\n")).expect("file");
    let (hits, result) = call(
        vec![script(200, r#"{"name":"demo"}"#)],
        "rig_start",
        json!({"sourceRef": path.display().to_string()}),
        false,
    )
    .await;
    assert!(hits.is_empty());
    let text = harness::text_of(&result);
    assert_eq!(result.is_error, Some(true));
    assert!(text.contains("not a rig spec"), "{text}");
    assert!(!text.contains(secret), "{text}");
    assert!(!text.contains("hunter2"), "{text}");
}

#[tokio::test]
async fn agent_read_screen_refuses_rig_or_pod() {
    let (hits, result) = call(
        vec![script(200, r#"{"text":"ready"}"#)],
        "agent_read_screen",
        json!({"session":"dev","rig":"demo"}),
        false,
    )
    .await;
    assert!(hits.is_empty());
    assert_eq!(harness::json_of(&result)["feature"], "rig");
    let (hits, result) = call(
        vec![script(200, r#"{"text":"ready"}"#)],
        "agent_read_screen",
        json!({"pod":"infra"}),
        false,
    )
    .await;
    assert!(hits.is_empty());
    assert_eq!(harness::json_of(&result)["feature"], "pod");
}

#[tokio::test]
async fn chat_send_ignores_sender() {
    let (hits, _) = call(
        vec![
            script(200, r#"[{"id":"r1","name":"demo"}]"#),
            script(201, r#"{"id":"m1","body":"hi"}"#),
        ],
        "chat_send",
        json!({"rigName":"demo","body":"hi","sender":"mcp"}),
        false,
    )
    .await;
    assert!(body(&hits[1]).get("sender").is_none());
    assert_eq!(body(&hits[1])["body"], "hi");
}

#[tokio::test]
async fn oversized_send_is_payload_rejected() {
    let (hits, result) = call(
        vec![close_early()],
        "agent_send",
        json!({"session":"dev","text": "x".repeat(512 * 1024)}),
        false,
    )
    .await;
    assert_eq!(result.is_error, Some(true), "{}", harness::text_of(&result));
    let text = harness::text_of(&result);
    assert!(text.contains("payload_rejected"), "{text}");
    assert!(!text.contains("Broken pipe"), "{text}");
    assert!(!text.to_ascii_lowercase().contains("broken pipe"), "{text}");
    drop(hits);
}

fn script(status: u16, body: &str) -> Scripted {
    Scripted {
        status,
        body: body.to_owned(),
        close_early: false,
    }
}

fn close_early() -> Scripted {
    Scripted {
        status: 200,
        body: String::new(),
        close_early: true,
    }
}

fn body(hit: &Hit) -> Value {
    serde_json::from_str(&hit.body).unwrap_or(Value::Null)
}

#[expect(
    clippy::expect_used,
    reason = "a failed tool call fails the test that asked for it"
)]
async fn call(
    scripted: Vec<Scripted>,
    name: &str,
    args: Value,
    as_seat: bool,
) -> (Vec<Hit>, rmcp::model::CallToolResult) {
    let daemon = FakeDaemon::start(scripted);
    let server = if as_seat {
        harness::seat(daemon.path(), SEAT, "tok")
    } else {
        harness::operator(daemon.path())
    };
    let session = Session::start(server).await;
    let result = session.call(name, args).await.expect(name);
    (daemon.hits(), result)
}
