//! Direct-delivery histogram, queue baton, and a crash before the queue commit.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use crewmux_types::LaunchPosture;
use crewmux_types::QItemId;
use serde_json::Value;

use crate::slice8_harness::{
    kill_sessions, open_on, restore, rig, screens, send, shutdown, spec, up,
};
use crate::state::App;

const SECRET: &str = "obligation-secret-xyz";

#[tokio::test]
async fn direct_crash_histogram_is_one_each() {
    let rig = rig().await;
    up(
        &rig.app,
        &spec(
            "trio",
            &[("alpha", "/tmp"), ("beta", "/tmp"), ("gamma", "/tmp")],
        ),
    )
    .await;
    let seats = ["alpha", "beta", "gamma"];
    let phases = [Some("before"), None, Some("ack")];
    let mut n = 0u32;
    for phase in phases {
        for _ in 0..2 {
            for seat in seats {
                rig.app.set_direct_phase(phase).await;
                let body = format!("m{n:02}");
                let sent = send(&rig.app, &format!("{seat}@trio"), &body).await;
                assert_eq!(
                    sent.get("status").and_then(Value::as_str),
                    Some("ok"),
                    "{sent}"
                );
                rig.mux.show_stub_prompt(&format!("{seat}__trio")).await;
                n += 1;
            }
        }
    }
    assert_eq!(n, 18);
    rig.app.set_direct_phase(None).await;
    let restored = restore(&rig.app, "trio", LaunchPosture::Auto).await;
    let actions = restored
        .get("reconcile")
        .and_then(Value::as_array)
        .expect("plan");
    let resend = actions
        .iter()
        .filter(|row| row.get("re_send").is_some())
        .count();
    let ack = actions
        .iter()
        .filter(|row| row.get("ack").is_some())
        .count();
    assert_eq!(resend, 6, "{actions:?}");
    assert_eq!(ack, 6, "{actions:?}");
    let mut hist = BTreeMap::new();
    for index in 0..18 {
        let body = format!("m{index:02}");
        let count = count_body(&rig.home, &body);
        *hist.entry(count).or_insert(0u32) += 1;
    }
    assert_eq!(hist.get(&1).copied(), Some(18), "{hist:?}");
}

#[tokio::test]
async fn queue_baton_survives_restart() {
    let rig = rig().await;
    up(&rig.app, &spec("trio", &[("alpha", "/tmp")])).await;
    post_item(&rig.app, "baton-1", SECRET).await;
    note_unconfirmed(&rig.app, "baton-1").await;
    let before = load_item(&rig.app, "baton-1").await;
    kill_sessions(&rig.app).await;
    let home = shutdown(rig).await;
    let store = Arc::new(crewmux_store::Store::open(&home.join("state.db")).expect("reopen"));
    let loaded = store.read(crewmux_store::load_book).expect("book");
    let rig = open_on(store, home).await;
    *rig.app.queue.lock().await = loaded;
    crate::reconcile::reconcile(&rig.app)
        .await
        .expect("reconcile");
    let restarted = load_item(&rig.app, "baton-1").await;
    assert_eq!(restarted, before);
    assert!(load_item(&rig.app, "baton-missing").await.is_none());
    restore(&rig.app, "trio", LaunchPosture::Auto).await;
    let after = load_item(&rig.app, "baton-1").await.expect("row");
    assert_eq!(after, before.expect("before"));
    let text = screens(&rig.app).await;
    assert!(!text.contains(SECRET), "{text}");
    assert!(text.contains("baton-1"), "{text}");
}

#[tokio::test]
async fn kill_daemon_mid_handoff_keeps_the_committed_row() {
    let rig = rig().await;
    up(&rig.app, &spec("trio", &[("alpha", "/tmp")])).await;
    post_item(&rig.app, "kept-1", SECRET).await;
    note_unconfirmed(&rig.app, "kept-1").await;
    let before = load_item(&rig.app, "kept-1").await.expect("kept");
    rig.app.fail_next_queue_save().await;
    let rejected = post_status(&rig.app, "lost-1", "should-not-commit").await;
    assert_ne!(rejected, StatusCode::OK);
    let home = shutdown(rig).await;
    let store = Arc::new(crewmux_store::Store::open(&home.join("state.db")).expect("reopen"));
    let loaded = store.read(crewmux_store::load_book).expect("book");
    assert!(loaded.get(&QItemId::new("lost-1").expect("id")).is_none());
    let rig = open_on(store, home).await;
    *rig.app.queue.lock().await = loaded;
    crate::reconcile::reconcile(&rig.app)
        .await
        .expect("reconcile");
    restore(&rig.app, "trio", LaunchPosture::Auto).await;
    let after = load_item(&rig.app, "kept-1").await.expect("kept");
    assert_eq!(after, before);
    let text = screens(&rig.app).await;
    assert!(!text.contains(SECRET), "{text}");
    assert!(!text.contains("should-not-commit"), "{text}");
}

async fn post_item(app: &App, id: &str, body: &str) {
    assert_eq!(post_status(app, id, body).await, StatusCode::OK);
}

async fn post_status(app: &App, id: &str, body: &str) -> StatusCode {
    let bytes = serde_json::json!({
        "id": id,
        "source": "alice@host",
        "destination": "alpha@trio",
        "body": body,
    })
    .to_string();
    crate::queue_http::create(State(app.clone()), Bytes::from(bytes))
        .await
        .status()
}

async fn note_unconfirmed(app: &App, id: &str) {
    let id = QItemId::new(id).expect("id");
    let now = crate::launch::now_ts();
    let snap = {
        let mut book = app.queue.lock().await;
        book.append_note(
            &id,
            "handed off, ack pending",
            Some("pointer"),
            Some("delivered-ack-pending"),
            now,
        )
        .expect("note");
        let snap = book.clone();
        drop(book);
        snap
    };
    app.commit_queue(snap).await.expect("commit");
}

async fn load_item(app: &App, id: &str) -> Option<ItemView> {
    let id = QItemId::new(id).expect("id");
    let store = Arc::clone(&app.store);
    let book = tokio::task::spawn_blocking(move || store.read(crewmux_store::load_book))
        .await
        .expect("join")
        .expect("load");
    book.get(&id).map(|item| ItemView {
        body: item.body.clone(),
        state: item.state.as_str().to_owned(),
        nudge: item.last_nudge.as_ref().map(|nudge| nudge.result.clone()),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ItemView {
    body: String,
    state: String,
    nudge: Option<String>,
}

fn count_body(home: &std::path::Path, body: &str) -> usize {
    let dir = home.join("transcripts").join("stub");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    let mut count = 0usize;
    for entry in entries.flatten() {
        let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
        count += text.matches(body).count();
    }
    count
}
