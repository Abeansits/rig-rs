//! `POST /api/send` takes the sender from the seat token, not the body.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crewmux_api::Client;
use crewmux_mux::{FakeMux, Multiplexer};
use crewmux_types::{SEAT_ID_HEADER, SEAT_TOKEN_HEADER};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult};
use serde_json::{Value, json};

use crate::flow::{harness_with, two_stub};
use crate::launch::bring_up;
use crate::state::App;

struct Harness {
    app: App,
    mux: Arc<FakeMux>,
    sock: PathBuf,
    dir: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.app.cancel.cancel();
        drop(std::fs::remove_file(&self.sock));
        drop(std::fs::remove_dir_all(&self.dir));
    }
}

async fn boot() -> Harness {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "rig-send-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("dir");
    let (app, mux) = harness_with(None, &dir, None).await;
    bring_up(&app, &two_stub("/tmp")).await.expect("up");
    let sock = dir.join("d.sock");
    let listener = crate::runtime::bind_socket(&sock).expect("bind");
    let served = app.clone();
    let cancel = app.cancel.clone();
    tokio::spawn(async move {
        drop(
            axum::serve(listener, crate::http::router(served))
                .with_graceful_shutdown(async move { cancel.cancelled().await })
                .await,
        );
    });
    let ready = exchange(&sock, "GET", "/healthz", None, Vec::new()).await;
    assert_eq!(ready.status, 200, "{}", ready.body);
    Harness {
        app,
        mux,
        sock,
        dir,
    }
}

fn proof(calls: &[Vec<String>], session: &str) -> (String, String) {
    let argv = calls
        .iter()
        .find(|argv| {
            argv.windows(2).any(|pair| {
                pair.first().map(String::as_str) == Some("-s")
                    && pair.get(1).map(String::as_str) == Some(session)
            })
        })
        .expect(session);
    let value = |prefix: &str| {
        argv.iter()
            .find_map(|arg| arg.strip_prefix(prefix))
            .unwrap_or_else(|| panic!("{prefix} missing"))
            .to_owned()
    };
    (value("RIG_SEAT_ID="), value("RIG_SEAT_TOKEN="))
}

fn headers(id: &str, token: &str) -> Vec<(String, String)> {
    vec![
        (SEAT_ID_HEADER.to_owned(), id.to_owned()),
        (SEAT_TOKEN_HEADER.to_owned(), token.to_owned()),
    ]
}

async fn post(
    sock: &std::path::Path,
    headers: Vec<(String, String)>,
    body: Value,
) -> crewmux_api::Response {
    let bytes = serde_json::to_vec(&body).expect("json");
    exchange(sock, "POST", "/api/send", Some(bytes), headers).await
}

async fn screen(app: &App) -> String {
    let shot = crate::query::capture(app, "one", None)
        .await
        .expect("capture");
    shot.get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

async fn exchange(
    sock: &std::path::Path,
    method: &str,
    path: &str,
    body: Option<Vec<u8>>,
    headers: Vec<(String, String)>,
) -> crewmux_api::Response {
    let sock = sock.to_path_buf();
    let method = method.to_owned();
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || {
        let client = Client::over_uds(sock);
        let view: Vec<(&str, &str)> = headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        client
            .request_with_headers(&method, &path, body.as_deref(), &view)
            .expect("http")
    })
    .await
    .expect("join")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn valid_seat_token_is_the_sender() {
    let harness = boot().await;
    let (id, token) = proof(&harness.mux.recorded().await, "one__pair");
    let response = post(
        &harness.sock,
        headers(&id, &token),
        json!({"to": "one@pair", "body": "hello-from-seat"}),
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body);
    let text = screen(&harness.app).await;
    assert!(text.contains("From: one@pair"), "{text}");
    assert!(text.contains("hello-from-seat"), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bogus_seat_token_is_refused() {
    let harness = boot().await;
    let (id, _) = proof(&harness.mux.recorded().await, "one__pair");
    let marker = "bogus-must-not-land";
    let before = screen(&harness.app).await;
    let response = post(
        &harness.sock,
        headers(&id, "not-the-token"),
        json!({"to": "one@pair", "body": marker, "from": "one@pair"}),
    )
    .await;
    assert_eq!(response.status, 400, "{}", response.body);
    assert!(
        response.body.contains("unknown sender"),
        "{}",
        response.body
    );
    let after = screen(&harness.app).await;
    assert_eq!(
        after
            .matches(marker)
            .count()
            .saturating_sub(before.matches(marker).count()),
        0,
        "{after}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn body_sender_does_not_override_the_token() {
    let harness = boot().await;
    let (id, token) = proof(&harness.mux.recorded().await, "one__pair");
    let response = post(
        &harness.sock,
        headers(&id, &token),
        json!({"to": "one@pair", "body": "claimed-elsewhere", "from": "mallory@pair"}),
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body);
    let text = screen(&harness.app).await;
    assert!(text.contains("From: one@pair"), "{text}");
    assert!(!text.contains("mallory"), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_token_is_operator_not_the_body() {
    let harness = boot().await;
    let response = post(
        &harness.sock,
        Vec::new(),
        json!({"to": "one@pair", "body": "unsigned", "from": "mallory@pair"}),
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body);
    let text = screen(&harness.app).await;
    assert!(text.contains("From: operator@host"), "{text}");
    assert!(!text.contains("mallory"), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replaced_stub_pane_receives_nothing_via_mcp() {
    let harness = boot().await;
    let (id, token) = proof(&harness.mux.recorded().await, "one__pair");
    let pane = one_pane(&harness.mux).await;
    let before = pastes(&harness.mux.recorded().await);
    harness
        .mux
        .set_pane_process(&pane.session, Some(99), &pane.current_cmd, None)
        .await;
    let marker = "mcp-replaced-pane-marker";
    let result = mcp_send(&harness.sock, &id, &token, marker).await;
    let text = tool_text(&result);
    assert_eq!(result.is_error, Some(true), "{text}");
    assert!(text.contains("lineage:pane_pid_mismatch"), "{text}");
    assert_eq!(pastes(&harness.mux.recorded().await), before);
    let shot = screen(&harness.app).await;
    assert_eq!(shot.matches(marker).count(), 0, "{shot}");
}

async fn one_pane(mux: &FakeMux) -> crewmux_mux::PaneInfo {
    mux.list_panes()
        .await
        .expect("panes")
        .into_iter()
        .find(|pane| pane.session.contains("one"))
        .expect("one")
}

fn pastes(calls: &[Vec<String>]) -> usize {
    calls
        .iter()
        .filter(|argv| argv.iter().any(|arg| arg == "paste-buffer"))
        .count()
}

fn tool_text(result: &CallToolResult) -> String {
    result
        .content
        .first()
        .and_then(|block| block.as_text())
        .map(|text| text.text.clone())
        .unwrap_or_default()
}

async fn mcp_send(sock: &std::path::Path, id: &str, token: &str, text: &str) -> CallToolResult {
    let caller = crewmux_mcp::caller_from_pair(Some(id), Some(token)).expect("seat");
    let server = crewmux_mcp::RigMcp::new(
        Client::over_uds(sock),
        caller,
        crewmux_mcp::SchemaMode::Native,
    );
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let server_task = tokio::spawn(async move {
        let running = server.serve(server_io).await.expect("serve");
        drop(running.waiting().await);
    });
    let client = ().serve(client_io).await.expect("client");
    let mut params = CallToolRequestParams::new("agent_send".to_owned());
    params.arguments = json!({"session": "one@pair", "text": text})
        .as_object()
        .cloned();
    let result = client.call_tool(params).await.expect("call");
    drop(client);
    server_task.abort();
    result
}
