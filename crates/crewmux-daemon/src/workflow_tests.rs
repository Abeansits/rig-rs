//! Boot sweep and the frontier guard over the daemon socket.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crewmux_adapters::{AdapterRegistry, StubAdapter};
use crewmux_api::Client;
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

async fn boot_app() -> Harness {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "rig-wf-{}-{}",
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
    let joined = tokio::task::spawn_blocking(move || {
        Client::over_uds(sock).request(&method, &path, body.as_deref())
    })
    .await
    .expect("join");
    match joined {
        Ok(response) => (response.status, response.body),
        Err(err) => panic!("exchange failed: {err}"),
    }
}

struct Lines(Arc<tokio::sync::Mutex<Vec<String>>>);

impl tracing::Subscriber for Lines {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_non_zero_u64(std::num::NonZeroU64::MIN)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        let mut msg = Note(None);
        event.record(&mut msg);
        if let Some(text) = msg.0
            && let Ok(mut slot) = self.0.try_lock()
        {
            slot.push(text);
        }
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

struct Note(Option<String>);

impl tracing::field::Visit for Note {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = Some(format!("{value:?}"));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.0 = Some(value.to_owned());
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn empty_boot_sweep_logs_the_noop_and_installs_builtins() {
    let harness = boot_app().await;
    let lines = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let subscriber = Lines(Arc::clone(&lines));
    let guard = tracing::subscriber::set_default(subscriber);
    let book = harness.app.queue.lock().await.clone();
    crate::workflow::boot(&harness.app, book)
        .await
        .expect("boot");
    drop(guard);
    let captured = lines.lock().await.clone();
    assert!(
        captured
            .iter()
            .any(|line| line.contains(crate::workflow::BOOT_EMPTY)),
        "{captured:?}"
    );
    let engine = harness.app.workflow.lock().await.clone();
    assert!(engine.instances.is_empty());
    assert!(engine.specs.contains_key("linear-build@1"));
}

#[tokio::test]
async fn live_frontier_close_is_refused_and_project_advances() {
    let harness = boot_app().await;
    let spec = harness.dir.join("linear.yaml");
    std::fs::write(
        &spec,
        "workflow:\n  id: mini\n  version: 1\n  roles:\n    planner:\n      preferred_targets: [a@rig]\n    builder:\n      preferred_targets: [b@rig]\n  steps:\n    - id: one\n      actor_role: planner\n    - id: two\n      actor_role: builder\n",
    )
    .expect("spec");
    let path = spec.display().to_string();
    let body = serde_json::to_vec(&json!({
        "specPath": path,
        "rootObjective": "ship",
        "createdBySession": "orch@rig",
    }))
    .expect("json");
    let (status, text) = call(
        &harness.sock,
        "POST",
        "/api/workflow/instantiate",
        Some(&body),
    )
    .await;
    assert_eq!(status, 201, "{text}");
    let created: Value = serde_json::from_str(&text).expect("created");
    let packet = created["entryQitemId"].as_str().expect("packet");
    let instance = created["instance"]["instanceId"].as_str().expect("id");
    let close = serde_json::to_vec(&json!({
        "id": packet,
        "actor": "a@rig",
        "state": "done",
    }))
    .expect("json");
    let (status, text) = call(&harness.sock, "POST", "/api/queue/update", Some(&close)).await;
    assert_eq!(status, 400, "{text}");
    let refused: Value = serde_json::from_str(&text).expect("refused");
    assert_eq!(refused["error"], "workflow_frontier_packet");
    assert!(
        refused["message"]
            .as_str()
            .unwrap_or("")
            .contains("rig workflow project")
    );
    let project = serde_json::to_vec(&json!({
        "instanceId": instance,
        "currentPacketId": packet,
        "exit": "handoff",
        "actorSession": "a@rig",
    }))
    .expect("json");
    let (status, text) = call(
        &harness.sock,
        "POST",
        "/api/workflow/project",
        Some(&project),
    )
    .await;
    assert_eq!(status, 200, "{text}");
    let projected: Value = serde_json::from_str(&text).expect("projected");
    assert_eq!(
        projected["emittedEventTypes"],
        json!([
            "queue.updated",
            "workflow.step_closed",
            "queue.created",
            "workflow.next_qitem_projected"
        ])
    );
    let (status, listed) = call(&harness.sock, "GET", "/api/workflow/list", None).await;
    assert_eq!(status, 200, "{listed}");
    let rows: Value = serde_json::from_str(&listed).expect("list");
    assert!(rows.as_array().is_some_and(|rows| rows.len() == 1));
}
