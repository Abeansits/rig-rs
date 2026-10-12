//! Health, stream, and lease routes over a private Unix socket.

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crewmux_adapters::{AdapterRegistry, StubAdapter};
use crewmux_api::Client;
use crewmux_health::{CandidateSet, ClassificationDecision, LabelField};
use crewmux_mux::{FakeMux, Multiplexer};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

struct Harness {
    app: crate::state::App,
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
        "rig-health-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("dir");
    let sock = dir.join("d.sock");
    let mux = Arc::new(FakeMux::with_socket("rig-rs"));
    mux.enable_stub_scripts().await;
    let store = Arc::new(crewmux_store::Store::open(std::path::Path::new(":memory:")).expect("db"));
    let shared = Arc::clone(&mux);
    let shared: Arc<dyn Multiplexer> = shared;
    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(StubAdapter::new(
        Arc::clone(&shared),
        "rig-stub-agent",
    )));
    let app = crate::state::App::assemble(
        store,
        shared,
        Arc::new(registry),
        None,
        CancellationToken::new(),
        dir.clone(),
    );
    app.launch_workers().await;
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
    tokio::task::yield_now().await;
    let ready = call(&sock, "GET", "/healthz", None).await;
    assert_eq!(ready.0, 200, "{}", ready.1);
    Harness { app, sock, dir }
}

async fn call(
    sock: &std::path::Path,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
) -> (u16, String) {
    let sock = sock.to_path_buf();
    let method = method.to_owned();
    let path = path.to_owned();
    let body = body.map(<[u8]>::to_vec);
    let mut last = String::new();
    for attempt in 0..8 {
        match once(sock.clone(), method.clone(), path.clone(), body.clone()).await {
            Ok(response) => return (response.status, response.body),
            Err(err) if safe_retry(&method, &err) => {
                last = err.to_string();
                if attempt < 7 {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
            Err(err) => panic!("exchange {method} {path}: {err}"),
        }
    }
    panic!("exchange {method} {path}: {last}");
}

async fn once(
    sock: PathBuf,
    method: String,
    path: String,
    body: Option<Vec<u8>>,
) -> Result<crewmux_api::Response, crewmux_api::Error> {
    tokio::task::spawn_blocking(move || {
        Client::over_uds(sock).request(&method, &path, body.as_deref())
    })
    .await
    .expect("join")
}

/// GET may be repeated. A POST is repeated only when dial itself failed.
///
/// An exchange error can mean the handler already committed. Archive is not
/// idempotent: repeating it turns a lost 200 into 404.
fn safe_retry(method: &str, err: &crewmux_api::Error) -> bool {
    if method == "GET" || method == "HEAD" {
        return true;
    }
    matches!(err, crewmux_api::Error::Connect { .. })
}

fn value(status: u16, body: &str) -> Value {
    assert_eq!(status, 200, "{body}");
    serde_json::from_str(body).expect("json")
}

#[tokio::test]
async fn health_routes_match_the_typescript_shapes() {
    let harness = boot().await;
    let (status, body) = call(&harness.sock, "GET", "/api/health", None).await;
    let list = value(status, &body);
    assert_eq!(list["schema"], "openrig.health-list/v0alpha1");
    assert_eq!(list["total"], 0);
    assert_eq!(list["limit"], 100);

    let (status, body) = call(&harness.sock, "GET", "/api/health-summary/gateway", None).await;
    assert_eq!(status, 503, "{body}");
    assert!(body.contains("gateway_subsystem_unavailable"));

    let (status, body) = call(&harness.sock, "GET", "/api/health-summary/version", None).await;
    assert_eq!(value(status, &body)["version"], "0.0.0");

    let (status, body) = call(&harness.sock, "GET", "/api/health-summary/nodes", None).await;
    let nodes = value(status, &body);
    assert_eq!(nodes["total"], 0);
    assert_eq!(nodes["attentionRequired"], 0);

    let (status, body) = call(&harness.sock, "GET", "/api/health-summary/context", None).await;
    let context = value(status, &body);
    assert_eq!(context["total"], 0);
    assert_eq!(context["byUrgency"]["unknown"], 0);

    let (status, body) = call(&harness.sock, "GET", "/api/health-diagnosis", None).await;
    let diagnosis = value(status, &body);
    assert_eq!(diagnosis["schema"], "openrig.health-diagnosis/v0alpha1");
    assert!(
        diagnosis["note"]
            .as_str()
            .unwrap_or("")
            .contains("absence is not evidence of health")
    );

    let (status, body) = call(&harness.sock, "GET", "/api/health-diagnosis/policy", None).await;
    assert_eq!(
        value(status, &body)["schema"],
        "openrig.health-policy/v0alpha1"
    );
}

#[tokio::test]
async fn context_sample_projects_a_warning_and_rejects_a_zero_limit() {
    let harness = boot().await;
    crate::launch::bring_up(
        &harness.app,
        "name: rig\nseats:\n  - name: seat\n    runtime: stub\n    cwd: /tmp\n",
    )
    .await
    .expect("up");
    let sample = json!({"seat": "seat@rig", "usedTokens": 97, "tokenLimit": 100});
    let bytes = serde_json::to_vec(&sample).expect("bytes");
    let (status, body) = call(&harness.sock, "POST", "/api/usage/sample", Some(&bytes)).await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = call(&harness.sock, "GET", "/api/health", None).await;
    let list = value(status, &body);
    assert_eq!(list["records"][0]["detector"], "context.pressure");
    assert_eq!(list["records"][0]["severity"], "warning");
    assert_eq!(list["records"][0]["schema"], "openrig.health/v0alpha1");

    let zero = json!({"seat": "seat@rig", "usedTokens": 1, "tokenLimit": 0});
    let bytes = serde_json::to_vec(&zero).expect("bytes");
    let (status, body) = call(&harness.sock, "POST", "/api/usage/sample", Some(&bytes)).await;
    assert_eq!(status, 400, "{body}");
    assert!(!body.contains("\"evaluated\":0"));

    let unknown = json!({"seat": "missing@rig", "usedTokens": 1, "tokenLimit": 10});
    let bytes = serde_json::to_vec(&unknown).expect("bytes");
    let (status, body) = call(&harness.sock, "POST", "/api/usage/sample", Some(&bytes)).await;
    assert_eq!(status, 404, "{body}");
    assert!(body.contains("seat not found"));
}

#[tokio::test]
async fn a_bad_usage_row_does_not_fail_the_read() {
    let harness = boot().await;
    crate::launch::bring_up(
        &harness.app,
        "name: rig\nseats:\n  - name: seat\n    runtime: stub\n    cwd: /tmp\n",
    )
    .await
    .expect("up");
    let store = std::sync::Arc::clone(&harness.app.store);
    tokio::task::spawn_blocking(move || {
        store.write(|conn| {
            conn.execute(
                "INSERT INTO context_usage (seat, used_tokens, token_limit, sampled_at_ms)
                 VALUES ('not a seat', 1, 2, 1)",
                [],
            )?;
            Ok(())
        })
    })
    .await
    .expect("join")
    .expect("row");
    let sample = json!({"seat": "seat@rig", "usedTokens": 97, "tokenLimit": 100});
    let bytes = serde_json::to_vec(&sample).expect("bytes");
    let (status, body) = call(&harness.sock, "POST", "/api/usage/sample", Some(&bytes)).await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = call(&harness.sock, "GET", "/api/health", None).await;
    let list = value(status, &body);
    assert_eq!(list["records"][0]["detector"], "context.pressure");
}

#[tokio::test]
async fn bad_policy_fails_the_query() {
    let harness = boot().await;
    let app = harness.app.clone();
    tokio::task::spawn_blocking(move || {
        app.store
            .write(|conn| crewmux_store::policy_store(conn, "not-json"))
    })
    .await
    .expect("join")
    .expect("store");
    let (status, body) = call(&harness.sock, "GET", "/api/health", None).await;
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("policy") || body.contains("expected"));
    assert!(!body.contains("openrig.health-list"));
}

#[tokio::test]
async fn stream_lease_and_classify_round_trip() {
    let harness = boot().await;
    let emit = json!({"id": "item-1", "body": "hello", "source": "alice@rig"});
    let bytes = serde_json::to_vec(&emit).expect("bytes");
    let (status, body) = call(&harness.sock, "POST", "/api/stream/emit", Some(&bytes)).await;
    assert_eq!(status, 201, "{body}");
    let (status, body) = call(&harness.sock, "GET", "/api/stream/list", None).await;
    let list = value(status, &body);
    assert_eq!(list[0]["id"], "item-1");
    assert_eq!(list[0]["streamItemId"], "item-1");

    let changed = serde_json::to_vec(&json!({"id": "item-1", "body": "other"})).expect("bytes");
    let (status, body) = call(&harness.sock, "POST", "/api/stream/emit", Some(&changed)).await;
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("different body"));
    let again = serde_json::to_vec(&json!({"id": "item-1", "body": "hello"})).expect("bytes");
    let (status, body) = call(&harness.sock, "POST", "/api/stream/emit", Some(&again)).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(value(status, &body)["body"], "hello");

    let (status, body) = call(
        &harness.sock,
        "POST",
        "/api/stream/item-1/archive",
        Some(b"{}"),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = call(&harness.sock, "GET", "/api/stream/list", None).await;
    assert_eq!(value(status, &body), json!([]));

    let acquire = serde_json::to_vec(&json!({"classifierSession": "alice@rig"})).expect("bytes");
    let (status, body) = call(
        &harness.sock,
        "POST",
        "/api/stream/lease/acquire",
        Some(&acquire),
    )
    .await;
    let lease = value(status, &body);
    assert_eq!(lease["state"], "active");
    assert_eq!(lease["classifierSession"], "alice@rig");
    let held = serde_json::to_vec(&json!({"classifierSession": "bob@rig"})).expect("bytes");
    let (status, body) = call(
        &harness.sock,
        "POST",
        "/api/stream/lease/acquire",
        Some(&held),
    )
    .await;
    assert_eq!(status, 409, "{body}");
    assert!(body.contains("lease_held"));

    let decision = ClassificationDecision::Abstain {
        reason: "not enough context".into(),
    };
    let candidates = CandidateSet {
        version: "v1".into(),
        values: [(LabelField::ClassificationType, ["note".into()].into())].into(),
        duplicate_candidates: Vec::new(),
    };
    let classify = json!({
        "streamItemId": "item-1",
        "classifierSession": "alice@rig",
        "leaseId": lease["leaseId"],
        "candidates": candidates,
        "decision": decision,
    });
    let bytes = serde_json::to_vec(&classify).expect("bytes");
    let (status, body) = call(&harness.sock, "POST", "/api/stream/classify", Some(&bytes)).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(value(status, &body)["recorded"], true);

    let live = serde_json::to_vec(&json!({"ifDead": true})).expect("bytes");
    let (status, body) = call(
        &harness.sock,
        "POST",
        "/api/stream/reclaim-classifier",
        Some(&live),
    )
    .await;
    assert_eq!(status, 409, "{body}");
    assert!(body.contains("lease_still_active"));
}

#[tokio::test]
async fn stream_watch_replays_and_resumes_on_the_event_hub() {
    let harness = boot().await;
    let emit = serde_json::to_vec(&json!({"id": "item-9", "body": "watch me"})).expect("bytes");
    let (status, body) = call(&harness.sock, "POST", "/api/stream/emit", Some(&emit)).await;
    assert_eq!(status, 201, "{body}");
    let sock = harness.sock.clone();
    let first = tokio::task::spawn_blocking(move || read_watch(&sock, "/api/stream/watch"))
        .await
        .expect("watch");
    assert!(first.contains("stream_emitted"), "{first}");
    assert!(first.contains("item-9"), "{first}");
    let id = first
        .lines()
        .find_map(|line| line.strip_prefix("id: "))
        .expect("sse id")
        .to_owned();
    let sock = harness.sock.clone();
    let resumed = tokio::task::spawn_blocking(move || {
        read_watch(&sock, &format!("/api/stream/watch?last_event_id={id}"))
    })
    .await
    .expect("resume");
    assert!(!resumed.contains("item-9"), "{resumed}");
}

fn read_watch(sock: &std::path::Path, path: &str) -> String {
    let mut stream = Client::over_uds(sock).open("GET", path, &[]).expect("open");
    assert_eq!(stream.status, 200, "{}", stream.content_type);
    assert!(stream.content_type.contains("text/event-stream"));
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .expect("timeout");
    let mut raw = Vec::new();
    let mut buf = [0_u8; 2048];
    loop {
        match Read::read(&mut stream, &mut buf) {
            Ok(0) => break,
            Ok(n) => raw.extend(buf.get(..n).unwrap_or(&[])),
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                break;
            }
            Err(err) => panic!("watch read: {err}"),
        }
    }
    String::from_utf8_lossy(&raw).into_owned()
}
