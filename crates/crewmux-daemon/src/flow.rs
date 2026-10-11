//! `FakeMux` journeys for up, ps, capture, attach, down, gates, and death.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use crewmux_adapters::{AdapterRegistry, StubAdapter};
use crewmux_core::EventLog;
use crewmux_mux::{FakeMux, Multiplexer};
use crewmux_store::Store;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::death::supervise;
use crate::launch::bring_up;
use crate::query::{attach_argv, capture, down, list_seats};
use crate::state::App;

async fn harness(script: Option<&str>) -> (App, Arc<FakeMux>) {
    let mux = Arc::new(FakeMux::with_socket("rig-rs"));
    mux.enable_stub_scripts().await;
    let store = Arc::new(Store::open(Path::new(":memory:")).expect("db"));
    let cloned = Arc::clone(&mux);
    let shared: Arc<dyn Multiplexer> = cloned;
    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(StubAdapter::new(
        Arc::clone(&shared),
        "rig-stub-agent",
    )));
    let app = App {
        store,
        mux: shared,
        socket_name: "rig-rs".to_owned(),
        stub_script: script.map(str::to_owned),
        cancel: CancellationToken::new(),
        activity: Arc::new(AtomicU64::new(0)),
        registry: Arc::new(registry),
    };
    tokio::spawn(supervise(app.clone()));
    (app, mux)
}

fn two_stub(cwd: &str) -> String {
    format!(
        "name: pair\nseats:\n  - name: one\n    runtime: stub\n    cwd: {cwd}\n  - name: two\n    runtime: stub\n    cwd: {cwd}\n"
    )
}

fn events_of(body: &Value) -> Vec<(String, Option<String>)> {
    body.get("events")
        .and_then(Value::as_array)
        .expect("events")
        .iter()
        .map(|event| {
            (
                event
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                event.get("seat").and_then(Value::as_str).map(str::to_owned),
            )
        })
        .collect()
}

fn flag(calls: &[Vec<String>], name: &str, value: &str) -> bool {
    calls.iter().any(|argv| {
        argv.windows(2).any(|pair| {
            pair.first().map(String::as_str) == Some(name)
                && pair.get(1).map(String::as_str) == Some(value)
        })
    })
}

fn seat_text(body: &Value, name: &str, field: &str) -> String {
    let seats = body
        .pointer("/rigs/0/seats")
        .and_then(Value::as_array)
        .expect("seats");
    let seat = seats
        .iter()
        .find(|seat| seat.get("name").and_then(Value::as_str) == Some(name))
        .expect(name);
    match seat.get(field) {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Bool(value)) => value.to_string(),
        _ => String::new(),
    }
}

#[tokio::test]
async fn up_two_stubs_are_ready_with_geometry_and_causal_events() {
    let (app, mux) = harness(None).await;
    let body = bring_up(&app, &two_stub("/tmp")).await.expect("up");
    assert_eq!(body.get("status").and_then(Value::as_str), Some("ready"));
    let events = events_of(&body);
    assert_eq!(
        events,
        vec![
            ("rig_created".to_owned(), None),
            ("seat_launched".to_owned(), Some("one".to_owned())),
            ("seat_launched".to_owned(), Some("two".to_owned())),
            ("seat_ready".to_owned(), Some("one".to_owned())),
            ("seat_ready".to_owned(), Some("two".to_owned())),
        ]
    );
    let calls = mux.recorded().await;
    assert!(flag(&calls, "-x", "90"), "{calls:?}");
    assert!(flag(&calls, "-y", "27"), "{calls:?}");
    assert!(mux.has_session("one__pair").await.expect("has"));
    assert!(mux.has_session("two__pair").await.expect("has"));
}

#[tokio::test]
async fn second_up_reports_already_running() {
    let (app, mux) = harness(None).await;
    let spec = two_stub("/tmp");
    bring_up(&app, &spec).await.expect("up");
    let before = mux.recorded().await.len();
    let again = bring_up(&app, &spec).await.expect("again");
    assert_eq!(
        again.get("status").and_then(Value::as_str),
        Some("already_running")
    );
    assert_eq!(mux.recorded().await.len(), before);
}

#[tokio::test]
async fn ps_is_unattached_with_only_the_control_client() {
    let (app, _mux) = harness(None).await;
    bring_up(&app, &two_stub("/tmp")).await.expect("up");
    let listed = list_seats(&app).await.expect("ps");
    assert_eq!(seat_text(&listed, "one", "attached"), "false");
    assert_eq!(seat_text(&listed, "one", "runtime"), "stub");
    assert_eq!(seat_text(&listed, "one", "tmux_session"), "one__pair");
    assert_eq!(seat_text(&listed, "one", "state"), "ready");
}

#[tokio::test]
async fn capture_contains_the_prompt_and_lines_are_bounded() {
    let (app, _mux) = harness(None).await;
    bring_up(&app, &two_stub("/tmp")).await.expect("up");
    let full = capture(&app, "one", None).await.expect("capture");
    let text = full.get("text").and_then(Value::as_str).unwrap_or_default();
    assert!(text.contains("stub> "), "{text}");
    let bounded = capture(&app, "one", Some(1)).await.expect("lines");
    assert_eq!(bounded.get("lines").and_then(Value::as_u64), Some(1));
}

#[tokio::test]
async fn attach_argv_uses_an_exact_session_target() {
    let (app, _mux) = harness(None).await;
    bring_up(&app, &two_stub("/tmp")).await.expect("up");
    let argv = attach_argv(&app, "one").await.expect("attach");
    assert_eq!(
        argv,
        vec![
            "tmux".to_owned(),
            "-L".to_owned(),
            "rig-rs".to_owned(),
            "attach".to_owned(),
            "-t".to_owned(),
            "=one__pair".to_owned(),
        ]
    );
    let missing = attach_argv(&app, "missing").await;
    assert!(missing.is_err());
}

#[tokio::test]
async fn gate_never_becomes_ready() {
    let (app, _mux) = harness(Some("print_gate=login_required")).await;
    let body = bring_up(&app, &two_stub("/tmp")).await.expect("up");
    assert_eq!(
        body.get("status").and_then(Value::as_str),
        Some("attention")
    );
    let kinds: Vec<_> = events_of(&body).into_iter().map(|(kind, _)| kind).collect();
    assert!(!kinds.iter().any(|kind| kind == "seat_ready"));
    let listed = list_seats(&app).await.expect("ps");
    assert_eq!(seat_text(&listed, "one", "state"), "attention");
    assert_eq!(seat_text(&listed, "one", "code"), "login_required");
}

#[tokio::test]
async fn exit_after_marks_the_seat_dead() {
    let (app, _mux) = harness(Some("exit_after_ms=30")).await;
    let spec = "name: pair\nseats:\n  - name: one\n    runtime: stub\n    cwd: /tmp\n";
    bring_up(&app, spec).await.expect("up");
    let mut dead = false;
    for _ in 0..40 {
        let listed = list_seats(&app).await.expect("ps");
        if seat_text(&listed, "one", "state") == "dead" {
            dead = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(dead, "seat stayed alive");
    let kinds = crate::db::read(&app.store, |conn| {
        Ok(EventLog::list(conn)?
            .into_iter()
            .map(|event| event.kind().to_owned())
            .collect::<Vec<_>>())
    })
    .await
    .expect("events");
    assert!(kinds.iter().any(|kind| kind == "seat_died"), "{kinds:?}");
}

#[tokio::test]
async fn down_kills_sessions_and_stops_seats() {
    let (app, mux) = harness(None).await;
    bring_up(&app, &two_stub("/tmp")).await.expect("up");
    let body = down(&app, None).await.expect("down");
    assert_eq!(body.get("sessions_killed").and_then(Value::as_u64), Some(2));
    assert!(!mux.has_session("one__pair").await.expect("one"));
    assert!(!mux.has_session("two__pair").await.expect("two"));
    let listed = list_seats(&app).await.expect("ps");
    assert_eq!(seat_text(&listed, "one", "state"), "stopped");
    assert_eq!(seat_text(&listed, "two", "state"), "stopped");
}

#[tokio::test]
async fn invalid_cwd_is_partial_and_keeps_the_other_seat() {
    let (app, mux) = harness(None).await;
    let spec = "\
name: pair
seats:
  - name: good
    runtime: stub
    cwd: /tmp
  - name: bad
    runtime: stub
    cwd: /tmp/crewmux-missing-cwd-slice1
";
    let body = bring_up(&app, spec).await.expect("up");
    assert_eq!(body.get("status").and_then(Value::as_str), Some("partial"));
    let listed = list_seats(&app).await.expect("ps");
    assert_eq!(seat_text(&listed, "good", "state"), "ready");
    assert_eq!(seat_text(&listed, "bad", "state"), "attention");
    assert_eq!(seat_text(&listed, "bad", "code"), "invalid_cwd");
    assert!(mux.has_session("good__pair").await.expect("good"));
    assert!(!mux.has_session("bad__pair").await.expect("bad"));
}
