//! Direct-delivery histogram, queue baton, and a crash before the queue commit.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use crewmux_core::{RigRepo, SeatRepo};
use crewmux_types::LaunchPosture;
use crewmux_types::QItemId;
use serde_json::Value;

use crate::slice8_harness::{
    kill_sessions, open_on, restore, rig, screens, send, shutdown, spec, up,
};
use crate::state::App;

const SECRET: &str = "obligation-secret-xyz";

#[tokio::test]
async fn failed_resend_stays_unacked() {
    let rig = rig().await;
    up(&rig.app, &spec("solo", &[("alpha", "/tmp")])).await;
    rig.app.set_direct_phase(Some("before")).await;
    let sent = send(&rig.app, "alpha@solo", "lost-body").await;
    assert_eq!(sent.get("status").and_then(Value::as_str), Some("ok"));
    kill_sessions(&rig.app).await;
    break_cwd(&rig.app, "solo").await;
    let first = restore(&rig.app, "solo", LaunchPosture::Auto).await;
    assert!(plan_has(&first, "re_send"), "{first}");
    assert!(!plan_has(&first, "ack"), "{first}");
    let text = screens(&rig.app).await;
    assert!(!text.contains("lost-body"), "{text}");
    let second = restore(&rig.app, "solo", LaunchPosture::Auto).await;
    assert!(plan_has(&second, "re_send"), "{second}");
    assert!(!plan_has(&second, "ack"), "{second}");
    let text = screens(&rig.app).await;
    assert!(!text.contains("lost-body"), "{text}");
}

#[tokio::test]
async fn startup_file_is_recorded_on_the_startup_channel() {
    let rig = rig().await;
    let dir = rig.home.join("startup");
    std::fs::create_dir_all(&dir).expect("startup dir");
    std::fs::write(dir.join("alpha.txt"), "boot-line\n").expect("startup");
    up(&rig.app, &spec("solo", &[("alpha", "/tmp")])).await;
    let rows = direct_rows(&rig.app, "solo").await;
    assert!(
        rows.iter()
            .any(|row| row.channel == "startup" && row.body == "boot-line"),
        "{rows:?}"
    );
}

fn plan_has(body: &Value, kind: &str) -> bool {
    body.get("reconcile")
        .and_then(Value::as_array)
        .is_some_and(|rows| rows.iter().any(|row| row.get(kind).is_some()))
}

async fn break_cwd(app: &App, name: &str) {
    let name = crewmux_types::RigName::new(name).expect("name");
    crate::db::write(&app.store, move |conn| {
        let rig = conn
            .get_by_name(&name)?
            .ok_or_else(|| crewmux_core::Error::RigNotFound(name.to_string()))?;
        for mut seat in conn.list_by_rig(&rig.id)? {
            seat.cwd = Some(std::path::PathBuf::from("/no/such/rig-slice8-cwd"));
            SeatRepo::upsert(conn, &seat)?;
        }
        Ok(())
    })
    .await
    .expect("cwd");
}

async fn direct_rows(app: &App, name: &str) -> Vec<crewmux_store::DirectRow> {
    let (rig_id, _) = crate::snapshot::load_rig(app, name).await.expect("rig");
    let id = rig_id.to_string();
    let store = Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || {
        store.read(move |conn| crewmux_store::list_direct(conn, &id))
    })
    .await
    .expect("join")
    .expect("rows")
}

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
    let missing: Vec<_> = (0..18)
        .filter(|index| count_body(&rig.home, &format!("m{index:02}")) == 0)
        .collect();
    assert_eq!(
        hist.get(&1).copied(),
        Some(18),
        "{hist:?} missing {missing:?}"
    );
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
    count_plain(&home.join("transcripts"), body)
}

fn count_plain(dir: &std::path::Path, body: &str) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut count = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            count += count_plain(&path, body);
            continue;
        }
        let plain = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"));
        if !plain {
            continue;
        }
        let contents = std::fs::read_to_string(&path).unwrap_or_default();
        count += contents.matches(body).count();
    }
    count
}
