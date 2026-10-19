//! The outbox loop: due filtering, exactly-once paste, cancel, and clock skew.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crewmux_adapters::{AdapterRegistry, StubAdapter};
use crewmux_core::Timestamp;
use crewmux_messaging::{NewItem, NudgeRecord, QueueBook, SessionRef};
use crewmux_mux::{FakeMux, Multiplexer};
use crewmux_orchestration::{Engine, Instance, Keepalive, Status};
use crewmux_store::{NewWatchJob, Store};
use crewmux_types::{QItemId, WorkflowInstanceId};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::health_query::write_db;
use crate::launch::bring_up;
use crate::outbox::tick;
use crate::state::App;

const T0: i64 = 1_700_000_000_000;

fn unique() -> u64 {
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

fn scratch(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    std::env::temp_dir().join(format!(
        "rig-outbox-{label}-{}-{}-{nanos}",
        std::process::id(),
        unique()
    ))
}

async fn open_app(path: &Path, home: PathBuf) -> (App, Arc<FakeMux>) {
    let mux = Arc::new(FakeMux::with_socket("rig-rs"));
    mux.enable_stub_scripts().await;
    let store = Arc::new(Store::open(path).expect("db"));
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
        home,
    );
    app.outbox.hold.store(true, Ordering::SeqCst);
    app.outbox.now_ms.store(T0, Ordering::SeqCst);
    app.launch_workers().await;
    (app, mux)
}

async fn attach(path: &Path, home: PathBuf, mux: Arc<FakeMux>) -> App {
    let store = Arc::new(Store::open(path).expect("db"));
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
        home,
    );
    app.outbox.hold.store(true, Ordering::SeqCst);
    app.outbox.now_ms.store(T0, Ordering::SeqCst);
    app.launch_workers().await;
    app
}

async fn held(path: &Path) -> (App, Arc<FakeMux>, PathBuf) {
    let home = scratch("mem");
    std::fs::create_dir_all(home.join("work")).expect("work");
    let (app, mux) = open_app(path, home.clone()).await;
    let cwd = home.join("work");
    bring_up(&app, &crate::flow::two_stub(cwd.to_str().expect("utf8")))
        .await
        .expect("up");
    (app, mux, home)
}

async fn memory() -> (App, Arc<FakeMux>, PathBuf) {
    held(Path::new(":memory:")).await
}

async fn shutdown(app: App) {
    app.cancel.cancel();
    app.tasks.close();
    tokio::time::timeout(Duration::from_secs(2), app.tasks.wait())
        .await
        .expect("workers");
}

async fn screen(app: &App, name: &str) -> String {
    let shot = crate::query::capture(app, name, None)
        .await
        .expect("capture");
    shot.get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

async fn put_job(app: &App, row: NewWatchJob) {
    write_db(app, move |conn| {
        crewmux_store::ensure_job(conn, &row).map(|_| ())
    })
    .await
    .expect("job");
}

fn reminder(
    id: &str,
    target: &str,
    message: &str,
    interval: i64,
    scan: Option<i64>,
) -> NewWatchJob {
    NewWatchJob {
        job_id: id.to_owned(),
        policy: "periodic-reminder".to_owned(),
        target_session: target.to_owned(),
        spec_yaml: format!(
            "policy: periodic-reminder\ntarget:\n  session: {target}\nmessage: {message}\n"
        ),
        message: None,
        interval_seconds: interval,
        scan_interval_seconds: scan,
        active_wake_interval_seconds: None,
        registered_at_ms: T0,
        registered_by: "ops@kernel".to_owned(),
    }
}

async fn stamp(app: &App, id: &str, at: i64) {
    let id = id.to_owned();
    write_db(app, move |conn| {
        crewmux_store::record_evaluation(conn, &id, at, true)
    })
    .await
    .expect("stamp");
}

async fn claim_status(app: &App) -> String {
    write_db(app, |conn| {
        let status = conn.query_row("SELECT status FROM watchdog_claims", [], |row| {
            row.get::<_, String>(0)
        })?;
        Ok(status)
    })
    .await
    .expect("claim status")
}

async fn history_len(app: &App, id: &str) -> usize {
    let id = id.to_owned();
    write_db(app, move |conn| crewmux_store::list_history(conn, &id))
        .await
        .expect("history")
        .len()
}

fn session(text: &str) -> SessionRef {
    SessionRef::new(text).expect("session")
}

fn instance(id: &str, status: Status, frontier: &str) -> Instance {
    Instance {
        id: WorkflowInstanceId::new(id).expect("id"),
        workflow_name: "wf".to_owned(),
        workflow_version: "1".to_owned(),
        created_by: session("creator@pair"),
        created_at: Timestamp::from_millis(0),
        status,
        frontier: vec![QItemId::new(frontier).expect("q")],
        current_step_id: None,
        hop_count: 0,
        fallback_synthesis: None,
        last_decision: None,
        completed_at: None,
        version: 1,
        resume_count: 0,
        hops_baseline: 0,
        bound_rig: None,
    }
}

async fn install(app: &App, engine: Engine, book: QueueBook) {
    *app.workflow.lock().await = engine;
    *app.queue.lock().await = book;
}

#[tokio::test]
async fn run_tick_now_only_evaluates_due_active_jobs() {
    let (app, _, _) = memory().await;
    put_job(&app, reminder("early", "one@pair", "m1", 30, None)).await;
    put_job(&app, reminder("late", "two@pair", "m2", 30, None)).await;
    stamp(&app, "early", T0 - 5_000).await;
    stamp(&app, "late", T0 - 60_000).await;
    tick(&app).await.expect("tick");
    assert!(!screen(&app, "one").await.contains("m1"));
    assert!(screen(&app, "two").await.contains("m2"));
}

#[tokio::test]
async fn run_tick_now_evaluates_a_due_job_at_every_subsequent_due_tick() {
    let (app, _, _) = memory().await;
    put_job(&app, reminder("again", "one@pair", "again-body", 1, None)).await;
    stamp(&app, "again", T0 - 5_000).await;
    tick(&app).await.expect("first");
    assert_eq!(screen(&app, "one").await.matches("again-body").count(), 1);
    app.outbox.now_ms.store(T0 + 2_000, Ordering::SeqCst);
    stamp(&app, "again", T0 + 2_000 - 5_000).await;
    tick(&app).await.expect("second");
    assert_eq!(screen(&app, "one").await.matches("again-body").count(), 2);
}

#[tokio::test]
async fn stopped_jobs_are_excluded_from_evaluation() {
    let (app, _, _) = memory().await;
    put_job(&app, reminder("stop", "one@pair", "stopped-body", 30, None)).await;
    stamp(&app, "stop", T0 - 60_000).await;
    write_db(&app, |conn| {
        crewmux_store::stop_job(conn, "stop", "operator")
    })
    .await
    .expect("stop");
    tick(&app).await.expect("tick");
    assert!(!screen(&app, "one").await.contains("stopped-body"));
}

#[tokio::test]
async fn terminal_jobs_are_excluded_from_evaluation() {
    let (app, _, _) = memory().await;
    put_job(
        &app,
        reminder("done", "one@pair", "terminal-body", 30, None),
    )
    .await;
    stamp(&app, "done", T0 - 60_000).await;
    write_db(&app, |conn| {
        crewmux_store::mark_terminal(conn, "done", "done")
    })
    .await
    .expect("terminal");
    tick(&app).await.expect("tick");
    assert!(!screen(&app, "one").await.contains("terminal-body"));
}

#[tokio::test]
async fn policy_evaluation_errors_are_caught_and_tick_continues_for_siblings() {
    let (app, _, _) = memory().await;
    let mut bad = reminder("bad", "one@pair", "unused", 30, None);
    bad.spec_yaml = "policy: periodic-reminder\ntarget:\n  session: one@pair\n".to_owned();
    put_job(&app, bad).await;
    put_job(&app, reminder("ok", "two@pair", "ok-body", 30, None)).await;
    stamp(&app, "bad", T0 - 60_000).await;
    stamp(&app, "ok", T0 - 60_000).await;
    tick(&app).await.expect("tick");
    assert!(screen(&app, "two").await.contains("ok-body"));
    assert_eq!(history_len(&app, "ok").await, 1);
    assert_eq!(history_len(&app, "bad").await, 0);
}

#[tokio::test]
async fn start_and_stop_are_idempotent() {
    let home = scratch("run");
    std::fs::create_dir_all(&home).expect("home");
    let (app, _) = open_app(Path::new(":memory:"), home).await;
    wait_running(&app, true).await;
    app.tasks.spawn(crate::outbox::run(app.clone()));
    tokio::task::yield_now().await;
    assert!(app.outbox.running.load(Ordering::SeqCst));
    app.cancel.cancel();
    app.tasks.close();
    tokio::time::timeout(Duration::from_secs(2), app.tasks.wait())
        .await
        .expect("stop");
    assert!(!app.outbox.running.load(Ordering::SeqCst));
    app.cancel.cancel();
}

async fn wait_running(app: &App, want: bool) {
    let start = std::time::Instant::now();
    while app.outbox.running.load(Ordering::SeqCst) != want {
        assert!(start.elapsed() < Duration::from_secs(2), "running flag");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn run_tick_now_does_not_invoke_policy_during_scan_window() {
    let (app, _, _) = memory().await;
    put_job(&app, reminder("scan", "one@pair", "counted", 30, Some(600))).await;
    stamp(&app, "scan", T0 - 45_000).await;
    let before = history_len(&app, "scan").await;
    tick(&app).await.expect("tick");
    assert!(!screen(&app, "one").await.contains("counted"));
    assert_eq!(history_len(&app, "scan").await, before);
    let stamped = write_db(&app, |conn| crewmux_store::get_job(conn, "scan"))
        .await
        .expect("get")
        .expect("row");
    assert_eq!(stamped.last_evaluation_at_ms, Some(T0 - 45_000));
}

#[tokio::test]
async fn run_tick_now_invokes_policy_when_scan_interval_elapsed() {
    let (app, _, _) = memory().await;
    put_job(&app, reminder("scan", "one@pair", "counted", 600, Some(30))).await;
    stamp(&app, "scan", T0 - 45_000).await;
    tick(&app).await.expect("tick");
    assert!(screen(&app, "one").await.contains("counted"));
    assert_eq!(history_len(&app, "scan").await, 1);
}

#[tokio::test]
async fn recovers_schedule_across_restart_via_sqlite() {
    let home = scratch("restart");
    std::fs::create_dir_all(home.join("work")).expect("work");
    let path = home.join("rig.sqlite");
    let (app, mux) = open_app(&path, home.clone()).await;
    bring_up(
        &app,
        &crate::flow::two_stub(home.join("work").to_str().expect("utf8")),
    )
    .await
    .expect("up");
    put_job(&app, reminder("persist", "one@pair", "persisted", 1, None)).await;
    stamp(&app, "persist", T0 - 5_000).await;
    shutdown(app).await;
    let app = attach(&path, home, mux).await;
    tick(&app).await.expect("restart");
    assert_eq!(screen(&app, "one").await.matches("persisted").count(), 1);
    tick(&app).await.expect("again");
    assert_eq!(screen(&app, "one").await.matches("persisted").count(), 1);
}

#[tokio::test]
async fn restart_while_sending_does_not_double_fire() {
    let home = scratch("send");
    std::fs::create_dir_all(home.join("work")).expect("work");
    let path = home.join("rig.sqlite");
    let (app, mux) = open_app(&path, home.clone()).await;
    bring_up(
        &app,
        &crate::flow::two_stub(home.join("work").to_str().expect("utf8")),
    )
    .await
    .expect("up");
    put_job(&app, reminder("crash", "one@pair", "once-body", 30, None)).await;
    stamp(&app, "crash", T0 - 60_000).await;
    app.outbox.pause_after_claim.store(true, Ordering::SeqCst);
    tick(&app).await.expect("claim");
    assert!(!screen(&app, "one").await.contains("once-body"));
    shutdown(app).await;
    let app = attach(&path, home, mux).await;
    tick(&app).await.expect("reconcile");
    assert!(!screen(&app, "one").await.contains("once-body"));
    assert_eq!(claim_status(&app).await, "indeterminate");
}

#[tokio::test]
async fn cancel_before_paste_does_not_deliver() {
    let (app, _, _) = memory().await;
    put_job(
        &app,
        reminder("cancel", "one@pair", "cancel-body", 30, None),
    )
    .await;
    stamp(&app, "cancel", T0 - 60_000).await;
    app.outbox.cancel_before_paste.store(true, Ordering::SeqCst);
    tick(&app).await.expect("cancel");
    assert!(!screen(&app, "one").await.contains("cancel-body"));
    tick(&app).await.expect("later");
    assert!(!screen(&app, "one").await.contains("cancel-body"));
    assert_eq!(claim_status(&app).await, "cancelled");
}

#[tokio::test]
async fn a_cancelled_token_does_not_paste() {
    let (app, _, _) = memory().await;
    put_job(&app, reminder("token", "one@pair", "token-body", 30, None)).await;
    stamp(&app, "token", T0 - 60_000).await;
    app.cancel.cancel();
    tick(&app).await.expect("tick");
    assert!(!screen(&app, "one").await.contains("token-body"));
}

#[tokio::test]
async fn clock_skew_backward_does_not_fire_and_forward_fires_once() {
    let (app, _, _) = memory().await;
    put_job(&app, reminder("skew", "one@pair", "skew-body", 30, None)).await;
    stamp(&app, "skew", T0).await;
    app.outbox.now_ms.store(T0 - 10_000, Ordering::SeqCst);
    tick(&app).await.expect("back");
    assert!(!screen(&app, "one").await.contains("skew-body"));
    app.outbox.now_ms.store(T0 + 30_000 * 10, Ordering::SeqCst);
    tick(&app).await.expect("forward");
    assert_eq!(screen(&app, "one").await.matches("skew-body").count(), 1);
    tick(&app).await.expect("same instant");
    assert_eq!(screen(&app, "one").await.matches("skew-body").count(), 1);
}

#[tokio::test]
async fn deadline_expiry_pastes_the_stuck_nudge_once() {
    let (app, _, _) = memory().await;
    let mut book = QueueBook::new();
    book.create(
        packet("q-dead", "one@pair", "secret-obligation"),
        Timestamp::from_millis(0),
    )
    .expect("packet");
    let row = instance("inst-dead", Status::Active, "q-dead");
    let engine = Engine {
        instances: vec![row],
        keepalives: vec![armed("inst-dead", true)],
        ..Engine::default()
    };
    install(&app, engine, book).await;
    app.outbox
        .now_ms
        .store(5 * 60 * 60 * 1000, Ordering::SeqCst);
    tick(&app).await.expect("due");
    let text = screen(&app, "one").await;
    assert_eq!(text.matches("Workflow deadline:").count(), 1);
    assert!(!text.contains("secret-obligation"));
    tick(&app).await.expect("again");
    assert_eq!(
        screen(&app, "one")
            .await
            .matches("Workflow deadline:")
            .count(),
        1
    );
}

#[tokio::test]
async fn operator_keepalive_pastes_the_trailer_once() {
    let (app, _, _) = memory().await;
    let mut book = QueueBook::new();
    book.create(
        packet("q-live", "one@pair", "secret-obligation"),
        Timestamp::from_millis(T0),
    )
    .expect("packet");
    let engine = Engine {
        instances: vec![instance("inst-live", Status::Active, "q-live")],
        keepalives: vec![armed("inst-live", false)],
        ..Engine::default()
    };
    install(&app, engine, book).await;
    tick(&app).await.expect("due");
    let text = screen(&app, "one").await;
    assert_eq!(text.matches("Workflow keepalive:").count(), 1);
    assert!(text.contains("Continue the current step"));
    assert!(!text.contains("secret-obligation"));
    tick(&app).await.expect("again");
    assert_eq!(
        screen(&app, "one")
            .await
            .matches("Workflow keepalive:")
            .count(),
        1
    );
}

#[tokio::test]
async fn periodic_reminder_prefers_the_job_message_column() {
    let (app, _, _) = memory().await;
    let mut row = reminder("col", "one@pair", "spec-body", 30, None);
    row.message = Some("column-body".to_owned());
    put_job(&app, row).await;
    stamp(&app, "col", T0 - 60_000).await;
    tick(&app).await.expect("tick");
    let text = screen(&app, "one").await;
    assert!(text.contains("column-body"));
    assert!(!text.contains("spec-body"));
}

#[tokio::test]
async fn deadline_gated_healthy_does_not_paste() {
    let (app, _, _) = memory().await;
    let mut book = QueueBook::new();
    book.create(
        packet("q-live", "one@pair", "body"),
        Timestamp::from_millis(T0),
    )
    .expect("packet");
    let engine = Engine {
        instances: vec![instance("inst-live", Status::Active, "q-live")],
        keepalives: vec![armed("inst-live", true)],
        ..Engine::default()
    };
    install(&app, engine, book).await;
    tick(&app).await.expect("tick");
    let text = screen(&app, "one").await;
    assert!(!text.contains("Workflow deadline:"));
    assert!(!text.contains("Workflow keepalive:"));
}

#[tokio::test]
async fn owner_wake_pastes_the_pointer_once() {
    let (app, _, _) = memory().await;
    let id = QItemId::new("q-owner").expect("id");
    let mut book = QueueBook::new();
    book.create(
        NewItem {
            id: Some(id.clone()),
            source: session("src@pair"),
            destination: session("one@pair"),
            body: "obligation-secret-body".to_owned(),
            priority: 0,
            tier: None,
            tags: Vec::new(),
            handed_off_from: None,
            last_nudge: Some(NudgeRecord {
                at: Timestamp::from_millis(T0),
                result: "recorded".to_owned(),
                text: "recorded".to_owned(),
            }),
            explicit_human: false,
            fingerprint: "fp".to_owned(),
        },
        Timestamp::from_millis(T0),
    )
    .expect("packet");
    let engine = Engine {
        instances: vec![instance("inst-owner", Status::Active, "q-owner")],
        ..Engine::default()
    };
    install(&app, engine, book).await;
    tick(&app).await.expect("wake");
    let text = screen(&app, "one").await;
    let pointer = format!("you have queue item {id}");
    assert_eq!(text.matches(pointer.as_str()).count(), 1);
    assert!(!text.contains("obligation-secret-body"));
    tick(&app).await.expect("again");
    assert_eq!(
        screen(&app, "one").await.matches(pointer.as_str()).count(),
        1
    );
}

fn packet(id: &str, dest: &str, body: &str) -> NewItem {
    NewItem {
        id: Some(QItemId::new(id).expect("id")),
        source: session("src@pair"),
        destination: session(dest),
        body: body.to_owned(),
        priority: 0,
        tier: None,
        tags: Vec::new(),
        handed_off_from: None,
        last_nudge: None,
        explicit_human: false,
        fingerprint: "fp".to_owned(),
    }
}

fn armed(instance_id: &str, gated: bool) -> Keepalive {
    let flag = if gated { "true" } else { "false" };
    Keepalive {
        instance_id: instance_id.to_owned(),
        target_session: "one@pair".to_owned(),
        interval_seconds: 60,
        deadline_gated: gated,
        armed: true,
        reason: None,
        yaml: format!(
            "policy: workflow-keepalive\ntarget:\n  session: one@pair\ncontext:\n  workflow_instance_id: {instance_id}\ndeadline_gated: {flag}\n"
        ),
    }
}
