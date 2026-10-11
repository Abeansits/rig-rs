//! Stub-runtime send, transcripts, the event backlog, and the queue baton.

use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crewmux_api::StreamKind;
use crewmux_core::SeatRepo;
use crewmux_messaging::{NewItem, QueueBook, QueueState, SessionRef};
use crewmux_types::QItemId;
use serde_json::Value;

use crate::deliver::{Outbound, deliver};
use crate::flow::{harness, harness_with, two_stub};
use crate::hub::{EventHub, Missed};
use crate::launch::bring_up;
use crate::state::App;

fn outbound(targets: &[&str], body: &str) -> Outbound {
    Outbound {
        from: "operator@host".to_owned(),
        targets: targets.iter().map(|target| (*target).to_owned()).collect(),
        rig: None,
        all: false,
        body: body.to_owned(),
        interact: false,
        reason: None,
        verify: false,
    }
}

fn paste_count(calls: &[Vec<String>]) -> usize {
    calls
        .iter()
        .filter(|argv| argv.iter().any(|arg| arg == "paste-buffer"))
        .count()
}

fn entered(calls: &[Vec<String>]) -> bool {
    calls
        .iter()
        .any(|argv| argv.iter().any(|arg| arg == "Enter"))
}

fn outcome_of(body: &Value) -> String {
    body.pointer("/receipts/0/outcome")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn dev_pair() -> String {
    "name: dev-pair-stub\nseats:\n  - name: dev-impl\n    runtime: stub\n    cwd: /tmp\n  - name: dev-qa\n    runtime: stub\n    cwd: /tmp\n".to_owned()
}

async fn ready(app: &App, spec: &str) {
    bring_up(app, spec).await.expect("up");
}

fn temp_home(label: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let path = std::env::temp_dir().join(format!("rig-s4-{label}-{nanos}"));
    std::fs::create_dir_all(&path).expect("home");
    path
}

#[tokio::test]
async fn keystroke_between_verdict_and_paste_is_not_sent() {
    let (app, mux) = harness(None).await;
    ready(&app, &two_stub("/tmp")).await;
    let before = paste_count(&mux.recorded().await);
    mux.keystroke_on_guard_read(2).await;
    let body = deliver(&app, outbound(&["one@pair"], "hello"))
        .await
        .expect("receipt");
    assert_eq!(outcome_of(&body), "deferred");
    assert_eq!(paste_count(&mux.recorded().await), before);
}

#[tokio::test]
async fn contaminated_paste_does_not_send_enter() {
    let (app, mux) = harness(None).await;
    ready(&app, &two_stub("/tmp")).await;
    mux.contaminate_on_paste(true).await;
    let body = deliver(&app, outbound(&["one@pair"], "hello"))
        .await
        .expect("receipt");
    assert_eq!(body.get("status").and_then(Value::as_str), Some("failed"));
    assert_eq!(outcome_of(&body), "contaminated");
    assert!(!entered(&mux.recorded().await));
    let Missed::Replay(events) = app.hub.missed(None).await else {
        panic!("backlog");
    };
    assert!(
        events
            .iter()
            .any(|event| event.kind == StreamKind::AttentionRaised)
    );
}

#[tokio::test]
async fn missing_payload_is_not_delivered() {
    let (app, mux) = harness(None).await;
    ready(&app, &two_stub("/tmp")).await;
    mux.swallow_paste(true).await;
    let err = deliver(&app, outbound(&["one@pair"], "hello"))
        .await
        .expect_err("missing");
    assert!(err.to_string().contains("did not render"), "{err}");
    assert!(!entered(&mux.recorded().await));
}

#[tokio::test]
async fn terminal_tap_defers_without_a_tmux_human() {
    let (app, mux) = harness(None).await;
    ready(&app, &two_stub("/tmp")).await;
    let before = paste_count(&mux.recorded().await);
    app.taps.record_name("one@pair", Instant::now());
    let body = deliver(&app, outbound(&["one@pair"], "hello"))
        .await
        .expect("receipt");
    assert_eq!(outcome_of(&body), "deferred");
    assert_eq!(paste_count(&mux.recorded().await), before);
}

#[tokio::test]
async fn send_renders_in_addressed_pane() {
    let (app, _mux) = harness(None).await;
    ready(&app, &dev_pair()).await;
    let marker = "s06-render-marker-4d9b";
    deliver(&app, outbound(&["dev-impl@dev-pair-stub"], marker))
        .await
        .expect("send");
    let shot = crate::query::capture(&app, "dev-impl", None)
        .await
        .expect("capture");
    let text = shot.get("text").and_then(Value::as_str).unwrap_or_default();
    assert!(text.contains("To: dev-impl@dev-pair-stub"), "{text}");
    assert!(text.contains(marker), "{text}");
    assert!(!text.contains("To: dev-qa@dev-pair-stub"), "{text}");
}

#[tokio::test]
async fn send_verify_means_rendered() {
    let (app, _mux) = harness(None).await;
    ready(&app, &dev_pair()).await;
    let mut message = outbound(&["dev-impl@dev-pair-stub"], "s06-render-marker-4d9b");
    message.verify = true;
    let body = deliver(&app, message).await.expect("verify");
    assert_eq!(body.get("status").and_then(Value::as_str), Some("ok"));
    assert_eq!(outcome_of(&body), "delivered");
    let shot = crate::query::capture(&app, "dev-impl", None)
        .await
        .expect("capture");
    let text = shot.get("text").and_then(Value::as_str).unwrap_or_default();
    assert!(text.contains("s06-render-marker-4d9b"), "{text}");
}

#[tokio::test]
async fn each_seat_gets_its_own_to_header() {
    let (app, _mux) = harness(None).await;
    ready(&app, &dev_pair()).await;
    let body = deliver(
        &app,
        outbound(
            &["dev-impl@dev-pair-stub", "dev-qa@dev-pair-stub"],
            "fan-out-body",
        ),
    )
    .await
    .expect("fan-out");
    assert_eq!(body.get("status").and_then(Value::as_str), Some("ok"));
    for seat in ["dev-impl", "dev-qa"] {
        let shot = crate::query::capture(&app, seat, None).await.expect(seat);
        let text = shot.get("text").and_then(Value::as_str).unwrap_or_default();
        assert!(
            text.contains(&format!("To: {seat}@dev-pair-stub")),
            "{text}"
        );
        assert!(text.contains("fan-out-body"), "{text}");
    }
}

#[tokio::test]
async fn one_failed_seat_does_not_block_the_other() {
    let (app, _mux) = harness(None).await;
    ready(&app, &two_stub("/tmp")).await;
    let body = deliver(&app, outbound(&["one@pair", "missing"], "hello"))
        .await
        .expect("partial");
    assert_eq!(body.get("status").and_then(Value::as_str), Some("partial"));
    let receipts = body
        .get("receipts")
        .and_then(Value::as_array)
        .expect("rows");
    assert_eq!(receipts.len(), 2);
    assert!(receipts.iter().any(|row| row.get("error").is_some()));
    assert!(receipts.iter().any(|row| {
        row.get("outcome").and_then(Value::as_str) == Some("delivered")
            && row.get("fingerprint").and_then(Value::as_str).is_some()
            && row.get("cursor").and_then(Value::as_u64).is_some()
    }));
}

#[tokio::test]
async fn interactive_gate_refuses_unless_dangerously_interact() {
    let (app, mux) = harness(Some("print_gate=trust_gate")).await;
    ready(&app, &two_stub("/tmp")).await;
    let before = paste_count(&mux.recorded().await);
    let err = deliver(&app, outbound(&["one@pair"], "hello"))
        .await
        .expect_err("refused");
    let text = err.to_string();
    assert!(
        text.contains("is at an interactive prompt (trust_gate)"),
        "{text}"
    );
    assert!(text.contains("No text was sent."), "{text}");
    assert!(
        text.contains("--dangerously-interact --reason \"<why>\""),
        "{text}"
    );
    assert_eq!(paste_count(&mux.recorded().await), before);
    let mut message = outbound(&["one@pair"], "driven");
    message.interact = true;
    let missing = deliver(&app, message).await.expect_err("reason");
    assert!(
        missing
            .to_string()
            .contains("--dangerously-interact requires --reason")
    );
    let mut driven = outbound(&["one@pair"], "driven");
    driven.interact = true;
    driven.reason = Some("the gate is the point".to_owned());
    let body = deliver(&app, driven).await.expect("override");
    assert_eq!(outcome_of(&body), "delivered");
    assert_eq!(
        body.pointer("/receipts/0/bracketed")
            .and_then(Value::as_bool),
        Some(false)
    );
    let paste = mux
        .recorded()
        .await
        .into_iter()
        .rev()
        .find(|argv| argv.iter().any(|arg| arg == "paste-buffer"))
        .expect("paste");
    assert!(!paste.iter().any(|arg| arg == "-p"), "{paste:?}");
}

#[tokio::test]
async fn queue_baton_survives_restart() {
    let (app, _mux) = harness(None).await;
    let id = QItemId::new("baton-1").expect("id");
    let dest = SessionRef::new("dev-qa@dev-pair-stub").expect("dest");
    let now = crate::launch::now_ts();
    {
        let mut book = app.queue.lock().await;
        book.create(baton(&id, &dest), now).expect("create");
        book.claim(&id, &dest, now, None).expect("claim");
        let snap = book.clone();
        drop(book);
        app.submit_queue(snap);
    }
    let stored = wait_claimed(&app, &id).await;
    assert_eq!(stored.state, QueueState::InProgress);
    assert_eq!(
        stored.claimed_by.as_ref().map(SessionRef::as_str),
        Some("dev-qa@dev-pair-stub")
    );
    let reloaded = load_book(&app).await;
    let item = reloaded.get(&id).expect("row");
    assert_eq!(item.state, QueueState::InProgress);
    assert_eq!(
        item.claimed_by.as_ref().map(SessionRef::as_str),
        Some("dev-qa@dev-pair-stub")
    );
}

#[tokio::test]
async fn transcript_writer_keeps_ansi_and_rotates() {
    let home = temp_home("transcripts");
    let (app, mux) = harness_with(None, &home, Some(4)).await;
    ready(&app, &two_stub("/tmp")).await;
    let pane = pane_of(&app, "one").await;
    mux.inject_output(&pane, "\u{1b}[31mneedle\u{1b}[0m\n")
        .await;
    let twin = wait_file(&home.join("transcripts/one/1.txt")).await;
    assert!(twin.contains("needle"), "{twin}");
    assert!(!twin.contains('\u{1b}'), "{twin}");
    let log = std::fs::read_to_string(home.join("transcripts/one/1.log")).expect("log");
    assert!(log.contains("\u{1b}[31m"), "{log}");
    mux.inject_output(&pane, "second-chunk\n").await;
    let rotated = wait_file(&home.join("transcripts/one/2.txt")).await;
    assert!(rotated.contains("second-chunk"), "{rotated}");
    drop(std::fs::remove_dir_all(&home));
}

#[tokio::test]
async fn resume_past_the_backlog_is_a_gap() {
    let hub = EventHub::new();
    for _ in 0..130 {
        hub.publish(StreamKind::Activity, None).await;
    }
    let Missed::Gap(events) = hub.missed(Some(1)).await else {
        panic!("expected a gap");
    };
    assert!(events.len() <= 128);
    assert!(events.first().is_some_and(|event| event.id > 1));
    let second = hub.publish(StreamKind::QueueChanged, None).await;
    let Missed::Replay(replay) = hub.missed(Some(second.id.saturating_sub(1))).await else {
        panic!("replay");
    };
    assert_eq!(replay.len(), 1);
    assert_eq!(replay.first().map(|event| event.id), Some(second.id));
}

fn baton(id: &QItemId, dest: &SessionRef) -> NewItem {
    NewItem {
        id: Some(id.clone()),
        source: SessionRef::new("dev-impl@dev-pair-stub").expect("source"),
        destination: dest.clone(),
        body: "hold the baton".to_owned(),
        priority: 0,
        tier: None,
        tags: Vec::new(),
        handed_off_from: None,
        last_nudge: None,
        explicit_human: false,
        fingerprint: String::new(),
    }
}

async fn load_book(app: &App) -> QueueBook {
    let store = std::sync::Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || store.read(crewmux_store::load_book))
        .await
        .expect("join")
        .expect("load")
}

async fn wait_claimed(app: &App, id: &QItemId) -> crewmux_messaging::QueueItem {
    for _ in 0..50 {
        if let Some(item) = load_book(app).await.get(id).cloned()
            && item.state == QueueState::InProgress
        {
            return item;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("baton was not persisted");
}

async fn pane_of(app: &App, name: &str) -> String {
    let seats = crate::db::read(&app.store, |conn| SeatRepo::list_all(conn))
        .await
        .expect("seats");
    seats
        .into_iter()
        .find(|seat| seat.name.as_str() == name)
        .and_then(|seat| seat.pane_id)
        .expect(name)
}

async fn wait_file(path: &Path) -> String {
    for _ in 0..50 {
        if let Ok(text) = std::fs::read_to_string(path) {
            return text;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("missing {}", path.display());
}
