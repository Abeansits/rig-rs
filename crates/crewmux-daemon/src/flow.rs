//! `FakeMux` journeys for up, ps, capture, attach, down, gates, and death.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crewmux_adapters::{AdapterRegistry, StubAdapter};
use crewmux_core::{EventLog, SeatRepo, SeatState};
use crewmux_mux::{FakeMux, Multiplexer};
use crewmux_store::Store;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::death::supervise;
use crate::launch::bring_up;
use crate::query::{attach_argv, capture, down, list_seats};
use crate::state::App;

pub(crate) async fn harness(script: Option<&str>) -> (App, Arc<FakeMux>) {
    let home = std::env::temp_dir().join(format!(
        "rig-flow-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos())
    ));
    std::fs::create_dir_all(&home).expect("flow home");
    harness_with(script, &home, None).await
}

pub(crate) async fn harness_with(
    script: Option<&str>,
    home: &Path,
    transcript_cap: Option<u64>,
) -> (App, Arc<FakeMux>) {
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
    let mut app = App::assemble(
        store,
        shared,
        Arc::new(registry),
        script.map(str::to_owned),
        CancellationToken::new(),
        home.to_path_buf(),
    );
    if let Some(cap) = transcript_cap {
        app.transcript_cap = cap;
    }
    app.launch_workers().await;
    tokio::spawn(supervise(app.clone()));
    (app, mux)
}

pub(crate) fn two_stub(cwd: &str) -> String {
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

fn seat<'a>(seats: &'a [crewmux_core::Seat], name: &str) -> &'a crewmux_core::Seat {
    seats
        .iter()
        .find(|seat| seat.name.as_str() == name)
        .expect(name)
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
async fn up_relaunches_a_dead_seat_beside_a_ready_one() {
    let (app, mux) = harness(None).await;
    let spec = two_stub("/tmp");
    bring_up(&app, &spec).await.expect("up");
    let before = crate::db::read(&app.store, |conn| conn.list_all())
        .await
        .expect("seats");
    let one_before = seat(&before, "one").clone();
    let two_before = seat(&before, "two").clone();
    crate::death::mark_dead(&app, &two_before, None, None)
        .await
        .expect("dead");
    assert!(mux.has_session("two__pair").await.expect("leftover"));
    let again = bring_up(&app, &spec).await.expect("again");
    assert_eq!(again.get("status").and_then(Value::as_str), Some("ready"));
    let events = events_of(&again);
    assert!(
        events
            .iter()
            .any(|(kind, name)| kind == "seat_launched" && name.as_deref() == Some("two")),
        "{events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|(_, name)| name.as_deref() == Some("one")),
        "{events:?}"
    );
    let after = crate::db::read(&app.store, |conn| conn.list_all())
        .await
        .expect("after");
    let one = seat(&after, "one");
    let two = seat(&after, "two");
    assert_eq!(one.id, one_before.id);
    assert_eq!(two.id, two_before.id);
    assert_eq!(one.state, SeatState::Ready);
    assert_eq!(two.state, SeatState::Ready);
    assert_eq!(one.pane_id, one_before.pane_id);
    assert_ne!(two.pane_id, two_before.pane_id);
    assert!(mux.has_session("one__pair").await.expect("one"));
    assert!(mux.has_session("two__pair").await.expect("two"));
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

/// PE-008: attach argv is `<tmux> -S <socket> attach -t =<session>`.
#[tokio::test]
async fn attach_argv_uses_an_exact_session_target() {
    let (app, _mux) = harness(None).await;
    bring_up(&app, &two_stub("/tmp")).await.expect("up");
    let argv = attach_argv(&app, "one").await.expect("attach");
    assert_eq!(
        argv,
        vec![
            "tmux".to_owned(),
            "-S".to_owned(),
            "/tmp/rig-fake-rig-rs".to_owned(),
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
    let died = crate::db::read(&app.store, |conn| {
        Ok(EventLog::list(conn)?
            .into_iter()
            .find(|event| event.kind() == "seat_died"))
    })
    .await
    .expect("events");
    let Some(crewmux_core::DomainEvent::SeatDied {
        exit_status,
        capture,
        ..
    }) = died
    else {
        panic!("seat_died missing");
    };
    assert_eq!(exit_status, Some(0));
    assert!(capture.is_some(), "seat_died records the last pane");
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

#[tokio::test]
async fn down_is_not_overwritten_by_death() {
    let (app, _mux) = harness(None).await;
    bring_up(&app, &two_stub("/tmp")).await.expect("up");
    let seats = crate::db::read(&app.store, |conn| conn.list_all())
        .await
        .expect("seats");
    let app_death = app.clone();
    let hammer = tokio::spawn(async move {
        for _ in 0..40 {
            for seat in &seats {
                crate::death::mark_dead(&app_death, seat, None, None)
                    .await
                    .expect("dead");
            }
            tokio::task::yield_now().await;
        }
    });
    down(&app, None).await.expect("down");
    hammer.await.expect("hammer");
    let listed = list_seats(&app).await.expect("ps");
    assert_eq!(seat_text(&listed, "one", "state"), "stopped");
    assert_eq!(seat_text(&listed, "two", "state"), "stopped");
}

#[tokio::test]
async fn three_parallel_ups_do_not_collide() {
    let (app, _mux) = harness(None).await;
    let spec = two_stub("/tmp");
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..3 {
        let app = app.clone();
        let spec = spec.clone();
        tasks.spawn(async move { bring_up(&app, &spec).await });
    }
    let mut ready = 0;
    let mut already = 0;
    while let Some(joined) = tasks.join_next().await {
        let body = joined.expect("join").expect("up");
        match body.get("status").and_then(Value::as_str) {
            Some("ready") => ready += 1,
            Some("already_running") => already += 1,
            other => panic!("status {other:?} body {body}"),
        }
    }
    assert_eq!(ready, 1);
    assert_eq!(already, 2);
}

#[tokio::test]
async fn restart_reconciles_a_half_launched_rig() {
    let (app, mux) = harness(None).await;
    bring_up(&app, &two_stub("/tmp")).await.expect("up");
    let seats = crate::db::read(&app.store, |conn| conn.list_all())
        .await
        .expect("seats");
    for seat in seats {
        let mut launching = seat;
        launching.state = SeatState::Launching;
        let saved = launching;
        crate::db::write(&app.store, move |conn| SeatRepo::upsert(conn, &saved))
            .await
            .expect("rewrite");
    }
    crate::reconcile::reconcile(&app).await.expect("reconcile");
    assert!(!mux.has_session("one__pair").await.expect("one"));
    assert!(!mux.has_session("two__pair").await.expect("two"));
    let again = bring_up(&app, &two_stub("/tmp")).await.expect("again");
    assert_eq!(again.get("status").and_then(Value::as_str), Some("ready"));
}

#[tokio::test]
async fn capture_rejects_the_same_seat_name_in_two_rigs() {
    let (app, _mux) = harness(None).await;
    bring_up(&app, &two_stub("/tmp")).await.expect("first");
    let other = "name: other\nseats:\n  - name: one\n    runtime: stub\n    cwd: /tmp\n";
    bring_up(&app, other).await.expect("second");
    let err = capture(&app, "one", None).await.expect_err("ambiguous");
    assert!(err.to_string().contains("ambiguous"), "{err}");
}

#[test]
fn launch_failures_keep_a_specific_code() {
    assert_eq!(
        crate::launch::attention_code("cwd does not exist: /missing"),
        "invalid_cwd"
    );
    assert_eq!(
        crate::launch::attention_code("permission denied"),
        "permission_denied"
    );
    assert_eq!(
        crate::launch::attention_code("no such file or directory"),
        "not_found"
    );
    assert_eq!(
        crate::launch::attention_code("adapter exploded"),
        "launch_failed"
    );
}
