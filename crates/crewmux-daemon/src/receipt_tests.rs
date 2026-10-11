//! Delivery-cursor restart, receipt write errors, and queue commit ordering.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crewmux_adapters::{AdapterRegistry, StubAdapter};
use crewmux_messaging::{NewItem, SessionRef};
use crewmux_mux::{FakeMux, Multiplexer};
use crewmux_store::Store;
use crewmux_types::QItemId;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::deliver::{Outbound, deliver};
use crate::flow::{harness, two_stub};
use crate::launch::bring_up;
use crate::state::App;

fn outbound() -> Outbound {
    Outbound {
        from: "operator@host".to_owned(),
        targets: vec!["one@pair".to_owned()],
        rig: None,
        all: false,
        body: "hello".to_owned(),
        interact: false,
        reason: None,
        verify: false,
    }
}

fn new_item(id: &QItemId, dest: &SessionRef) -> NewItem {
    NewItem {
        id: Some(id.clone()),
        source: SessionRef::new("alice@rig").expect("source"),
        destination: dest.clone(),
        body: "acked".to_owned(),
        priority: 0,
        tier: None,
        tags: Vec::new(),
        handed_off_from: None,
        last_nudge: None,
        explicit_human: false,
        fingerprint: String::new(),
    }
}

async fn load(app: &App) -> crewmux_messaging::QueueBook {
    let store = Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || store.read(crewmux_store::load_book))
        .await
        .expect("join")
        .expect("load")
}

async fn app_with(store: Arc<Store>, home: &std::path::Path) -> (App, Arc<FakeMux>) {
    let mux = Arc::new(FakeMux::with_socket("rig-rs"));
    mux.enable_stub_scripts().await;
    let cloned = Arc::clone(&mux);
    let shared: Arc<dyn Multiplexer> = cloned;
    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(StubAdapter::new(
        Arc::clone(&shared),
        "rig-stub-agent",
    )));
    let app = App::assemble(
        store,
        shared,
        Arc::new(registry),
        None,
        CancellationToken::new(),
        home.to_path_buf(),
    );
    app.launch_workers().await;
    (app, mux)
}

#[tokio::test]
async fn receipt_write_error_is_surfaced() {
    let (app, _mux) = harness(None).await;
    bring_up(&app, &two_stub("/tmp")).await.expect("up");
    let store = Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || {
        store.write(|conn| crewmux_store::insert_receipt(conn, 1, "{}"))
    })
    .await
    .expect("join")
    .expect("insert");
    let err = deliver(&app, outbound()).await.expect_err("collision");
    assert!(err.to_string().contains("receipt:"), "{err}");
}

#[tokio::test]
async fn receipt_cursor_survives_restart() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let home = std::env::temp_dir().join(format!("rig-s4-cursor-{nanos}"));
    std::fs::create_dir_all(&home).expect("home");
    let db = home.join("rig.db");
    let store = Arc::new(Store::open(&db).expect("db"));
    store
        .write(|conn| crewmux_store::insert_receipt(conn, 4, "{}"))
        .expect("seed");
    drop(store);

    let store = Arc::new(Store::open(&db).expect("reopen"));
    let max = store.read(crewmux_store::max_receipt_cursor).expect("max");
    assert_eq!(max, 4);
    let (app, _mux) = app_with(store, &home).await;
    app.seed_cursor(max);
    bring_up(&app, &two_stub("/tmp")).await.expect("up");
    let body = deliver(&app, outbound()).await.expect("send");
    assert_eq!(
        body.pointer("/receipts/0/cursor").and_then(Value::as_u64),
        Some(5)
    );
    app.cancel.cancel();
    app.tasks.close();
    app.tasks.wait().await;
    drop(app);

    let store = Store::open(&db).expect("third");
    assert_eq!(
        store.read(crewmux_store::max_receipt_cursor).expect("max"),
        5
    );
    drop(store);
    drop(std::fs::remove_dir_all(&home));
}

#[tokio::test]
async fn queue_commit_is_durable_before_the_ack() {
    let (app, _mux) = harness(None).await;
    let id = QItemId::new("ack-1").expect("id");
    let dest = SessionRef::new("bob@rig").expect("dest");
    let now = crate::launch::now_ts();
    let snap = {
        let mut book = app.queue.lock().await;
        book.create(new_item(&id, &dest), now).expect("create");
        let snap = book.clone();
        drop(book);
        snap
    };
    app.commit_queue(snap).await.expect("commit");
    let loaded = load(&app).await;
    assert_eq!(
        loaded.get(&id).map(|item| item.body.as_str()),
        Some("acked")
    );
}

#[tokio::test]
async fn cancel_flushes_a_queued_snapshot() {
    let (app, _mux) = harness(None).await;
    let id = QItemId::new("flush-1").expect("id");
    let dest = SessionRef::new("bob@rig").expect("dest");
    let now = crate::launch::now_ts();
    {
        let mut book = app.queue.lock().await;
        book.create(new_item(&id, &dest), now).expect("create");
        let snap = book.clone();
        drop(book);
        app.submit_queue(snap);
    }
    app.cancel.cancel();
    app.tasks.close();
    app.tasks.wait().await;
    let loaded = load(&app).await;
    assert!(loaded.get(&id).is_some(), "snapshot dropped on cancel");
}
