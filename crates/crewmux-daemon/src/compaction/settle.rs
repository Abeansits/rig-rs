//! A finished or cancelled manual compact does not stick the seat on `compacting`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crewmux_adapters::{AdapterRegistry, ClaudeAdapter};
use crewmux_core::{Rig, RigRepo, Seat, SeatRepo, SeatState, Timestamp};
use crewmux_health::compaction::{
    AttemptState, CompactTrigger, HookEvent, ManualCompactionStage, PrepMode, PrepStatus,
    PreparationState, SessionSource,
};
use crewmux_mux::{Capture, FakeMux, Multiplexer};
use crewmux_types::{RigId, RigName, RuntimeKind, SeatId, SeatName};
use tokio_util::sync::CancellationToken;

use super::trigger::{cancel, release_finished, status, trigger};
use crate::db::write;
use crate::state::App;

#[tokio::test]
async fn compact_after_abort_sends_again() {
    let rig = boot("abort", Duration::from_millis(800)).await;
    watch(Arc::clone(&rig.mux), rig.transcript.clone());
    let body = br#"{"seat":"claude","skip_map":true}"#;
    let first = trigger(&rig.app, body).await.expect("first compact");
    assert_eq!(first["compact_sent"], true, "{first}");
    {
        let mut guard = rig.app.compaction.lock().await;
        let now = crate::state::now_ms();
        assert!(guard.book.tick(now.saturating_add(120_000)));
        assert!(release_finished(&mut guard));
        assert!(!guard.prep("claude").active);
    }
    let second = trigger(&rig.app, body).await.expect("compact after abort");
    assert_eq!(second["compact_sent"], true, "{second}");
    drop(std::fs::remove_dir_all(&rig.root));
}

#[tokio::test]
async fn refused_precheck_fails_the_attempt_and_the_next_compact_sends_again() {
    let rig = boot("failed-prep", Duration::from_millis(800)).await;
    rig.mux
        .set_capture(Capture::new("booting\n".to_owned(), (0, 0)))
        .await;
    let body = br#"{"seat":"claude"}"#;
    let failed = trigger(&rig.app, body)
        .await
        .expect_err("draft refuses the compact");
    assert!(failed.to_string().contains("send_failed"), "{failed}");
    let reported = status(&rig.app, body).await.expect("status");
    assert_eq!(reported["stage"], "failed", "{reported}");
    let attempt = &reported["attempts"][0];
    assert_eq!(attempt["state"], "failed", "{reported}");
    assert_eq!(attempt["abort_reason"], "send_failed", "{reported}");
    let first_id = attempt["attempt_id"].as_str().expect("id").to_owned();
    assert!(first_id.ends_with("-1"), "{first_id}");
    {
        let guard = rig.app.compaction.lock().await;
        assert!(!guard.prep("claude").active);
    }
    let rule = "─".repeat(40);
    rig.mux
        .set_capture(Capture::new(
            format!("{rule}\n❯ \n{rule}\n  ? for shortcuts\n"),
            (0, 0),
        ))
        .await;
    watch(Arc::clone(&rig.mux), rig.transcript.clone());
    let second = trigger(&rig.app, body)
        .await
        .expect("compact after a refused pre-check");
    assert_eq!(second["compact_sent"], false, "{second}");
    assert!(
        !second.to_string().contains("preparation_pending"),
        "{second}"
    );
    let guard = rig.app.compaction.lock().await;
    let prep = guard.prep("claude");
    assert!(prep.active, "{prep:?}");
    assert_ne!(prep.attempt_id, first_id);
    assert!(prep.attempt_id.ends_with("-2"), "{}", prep.attempt_id);
    drop(guard);
    drop(std::fs::remove_dir_all(&rig.root));
}

#[tokio::test]
async fn contaminated_prep_fails_the_attempt() {
    let rig = boot("contaminated", Duration::from_millis(800)).await;
    watch_once_contaminated(Arc::clone(&rig.mux), rig.transcript.clone());
    let body = br#"{"seat":"claude"}"#;
    let failed = trigger(&rig.app, body)
        .await
        .expect_err("transcript did not match");
    let message = failed.to_string();
    assert!(
        message.contains("transcript did not confirm") || message.contains("merged the paste"),
        "{message}"
    );
    let reported = status(&rig.app, body).await.expect("status");
    assert_eq!(reported["stage"], "failed", "{reported}");
    let attempt = &reported["attempts"][0];
    assert_eq!(attempt["state"], "failed", "{reported}");
    assert_eq!(attempt["abort_reason"], message, "{reported}");
    let id = attempt["attempt_id"].as_str().expect("id").to_owned();
    let seat_key = attempt["seat_key"].as_str().expect("seat").to_owned();
    {
        let mut guard = rig.app.compaction.lock().await;
        assert!(!guard.prep("claude").active);
        let session = "11111111-1111-4111-8111-111111111111";
        guard.book.on_hook(&seat_key, &precompact(session), 10);
        guard.book.on_hook(&seat_key, &compact_start(session), 11);
        guard.book.on_hook(&seat_key, &post_compact(session), 12);
        let still = guard
            .book
            .attempts()
            .iter()
            .find(|row| row.id.as_str() == id)
            .expect("attempt");
        assert!(
            matches!(still.state, AttemptState::Failed(_)),
            "{:?}",
            still.state
        );
    }
    drop(std::fs::remove_dir_all(&rig.root));
}

#[test]
fn live_compact_captures_are_the_pre_fix_failures() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/pr18");
    for (name, sha) in [
        (
            "compact1.json",
            "afc32c63ed2941337c1f120fe064fe4cfc621212d30338862776ce99a7b2fab5",
        ),
        (
            "compact3.json",
            "ae6063510105afb8dfd04e7cf1d72d703badd4c2b1cff6a52280322fb00979e6",
        ),
        (
            "compact-status-late.json",
            "3d4bc81324adf1e7604c53bc8d3b09ed8285c868c1666e4653163ffebf18dafd",
        ),
    ] {
        let text = std::fs::read_to_string(dir.join(name)).expect(name);
        assert!(!text.contains("/Users/"), "{name}");
        assert_eq!(crewmux_transcript::fingerprint_raw(&text), sha, "{name}");
    }
    let pending = std::fs::read_to_string(dir.join("compact3.json")).expect("compact3");
    assert!(pending.contains("preparation_pending"), "{pending}");
}

#[tokio::test]
async fn paste_mismatch_fails_the_prep_and_the_next_compact_sends_again() {
    let rig = boot("paste-miss", Duration::from_millis(800)).await;
    rig.mux
        .set_capture_after_paste(Capture::new("nope\n".to_owned(), (0, 0)))
        .await;
    let body = br#"{"seat":"claude"}"#;
    let failed = trigger(&rig.app, body)
        .await
        .expect_err("paste did not land");
    let message = failed.to_string();
    assert!(
        message.contains("composer did not show the paste"),
        "{message}"
    );
    let reported = status(&rig.app, body).await.expect("status");
    assert_eq!(reported["stage"], "failed", "{reported}");
    let attempt = &reported["attempts"][0];
    assert_eq!(attempt["state"], "failed", "{reported}");
    assert_eq!(attempt["abort_reason"], "paste_mismatch", "{reported}");
    let first_id = attempt["attempt_id"]
        .as_str()
        .expect("attempt id")
        .to_owned();
    assert!(
        !reported.to_string().contains("preparation_pending"),
        "{reported}"
    );

    let rule = "─".repeat(40);
    rig.mux
        .set_capture(Capture::new(
            format!("{rule}\n❯ \n{rule}\n  ? for shortcuts\n"),
            (0, 0),
        ))
        .await;
    // The missed paste is already in `last_paste`. Recording it as well as the
    // next prep makes two user turns, and confirm waits out the budget.
    let already = rig.mux.last_paste().await.unwrap_or_default();
    watch_after(Arc::clone(&rig.mux), rig.transcript.clone(), already);
    let second = trigger(&rig.app, body)
        .await
        .expect("compact after a missed paste");
    let again = second.to_string();
    assert!(!again.contains("preparation_pending"), "{second}");
    let pasted = rig.mux.last_paste().await.expect("second paste");
    assert!(pasted.contains("OpenRig manual compaction"), "{pasted}");
    let guard = rig.app.compaction.lock().await;
    let prep = guard.prep("claude");
    assert!(prep.active, "{prep:?}");
    assert_ne!(prep.attempt_id, first_id);
    drop(guard);
    drop(std::fs::remove_dir_all(&rig.root));
}

#[tokio::test]
async fn compact_after_cancel_sends_again() {
    let rig = boot("cancel", Duration::from_millis(800)).await;
    watch(Arc::clone(&rig.mux), rig.transcript.clone());
    let body = br#"{"seat":"claude","skip_map":true}"#;
    let first = trigger(&rig.app, body).await.expect("first compact");
    assert_eq!(first["compact_sent"], true, "{first}");
    let cancelled = cancel(&rig.app, br#"{"seat":"claude"}"#)
        .await
        .expect("cancel");
    assert_eq!(cancelled["state"], "cancelled");
    let second = trigger(&rig.app, body).await.expect("compact after cancel");
    assert_eq!(second["compact_sent"], true, "{second}");
    assert!(
        !second.to_string().contains("preparation_stopped"),
        "{second}"
    );
    drop(std::fs::remove_dir_all(&rig.root));
}

#[tokio::test]
async fn cancel_during_restore_drops_the_pending_prompt() {
    let rig = boot("restore-cancel", Duration::from_millis(800)).await;
    watch(Arc::clone(&rig.mux), rig.transcript.clone());
    let body = br#"{"seat":"claude","skip_map":true}"#;
    let session = "11111111-1111-4111-8111-111111111111";
    let first = trigger(&rig.app, body).await.expect("first compact");
    assert_eq!(first["compact_sent"], true, "{first}");
    let seat_key = {
        let guard = rig.app.compaction.lock().await;
        guard
            .book
            .attempts()
            .first()
            .expect("attempt")
            .seat_key
            .clone()
    };
    {
        let mut guard = rig.app.compaction.lock().await;
        guard.book.on_hook(&seat_key, &precompact(session), 1);
        guard.book.on_hook(&seat_key, &compact_start(session), 2);
        guard.book.on_hook(&seat_key, &post_compact(session), 3);
        guard.enforcer.note_hooks_completed("claude");
        let attempt = guard
            .book
            .for_seat(&seat_key)
            .into_iter()
            .next_back()
            .expect("completed");
        assert!(matches!(attempt.state, AttemptState::Completed { .. }));
        assert!(
            guard
                .book
                .due_restore(&seat_key, true, crate::state::now_ms())
                .is_some()
        );
        assert_eq!(
            guard.enforcer.manual_stage("claude"),
            Some(ManualCompactionStage::Restoring)
        );
    }
    let cancelled = cancel(&rig.app, br#"{"seat":"claude"}"#)
        .await
        .expect("cancel");
    assert_eq!(cancelled["state"], "cancelled");
    let before = rig.mux.last_paste().await;
    super::apply::ingest(&rig.app, &seat_key, stop_hook(session).as_bytes())
        .await
        .expect("stop");
    assert_eq!(
        rig.mux.last_paste().await,
        before,
        "a cancelled restore was pasted on Stop"
    );
    let second = trigger(&rig.app, body)
        .await
        .expect("compact after a cancelled restore");
    assert_eq!(second["compact_sent"], true, "{second}");
    drop(std::fs::remove_dir_all(&rig.root));
}

#[tokio::test]
async fn skip_map_waits_until_the_composer_is_idle() {
    let rig = boot("idle-wait", Duration::from_millis(800)).await;
    watch(Arc::clone(&rig.mux), rig.transcript.clone());
    rig.mux
        .set_capture_after_enter(Capture::new("booting\n".to_owned(), (0, 0)))
        .await;
    let mux = Arc::clone(&rig.mux);
    let pane = crewmux_mux::PaneRef::session("seat__rig");
    tokio::spawn(async move {
        for _ in 0..400 {
            let shot = mux
                .capture(&pane, crewmux_mux::CaptureRange::Visible)
                .await
                .unwrap_or_else(|_| Capture::new(String::new(), (0, 0)));
            if shot.text.contains("booting") {
                tokio::time::sleep(Duration::from_millis(120)).await;
                let rule = "─".repeat(40);
                mux.set_capture(Capture::new(
                    format!("{rule}\n❯ \n{rule}\n  ? for shortcuts\n"),
                    (0, 0),
                ))
                .await;
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    });
    let value = trigger(&rig.app, br#"{"seat":"claude","skip_map":true}"#)
        .await
        .expect("compact after the idle wait");
    assert_eq!(value["compact_sent"], true, "{value}");
    drop(std::fs::remove_dir_all(&rig.root));
}

#[tokio::test]
async fn skip_map_idle_timeout_fails_the_attempt_so_the_next_compact_sends() {
    let rig = boot("idle-timeout", Duration::from_millis(800)).await;
    watch(Arc::clone(&rig.mux), rig.transcript.clone());
    let rule = "─".repeat(40);
    let ready = Capture::new(format!("{rule}\n❯ \n{rule}\n  ? for shortcuts\n"), (0, 0));
    let seat = crate::query::find_seat(&rig.app, "claude")
        .await
        .expect("seat");
    {
        let mut guard = rig.app.compaction.lock().await;
        let id = guard.book.next_manual_id(&seat.id.to_string());
        guard.set_prep(
            "claude",
            PreparationState {
                active: true,
                attempt_id: id.0,
                mode: PrepMode::Manual,
                status: PrepStatus::Waiting,
                map_complete: true,
                paste_accepted: true,
                deadline_ms: Some(crate::state::now_ms().saturating_add(150)),
                reason: None,
            },
        );
    }
    rig.mux
        .script_captures(vec![
            ready.clone(),
            Capture::new("booting\n".to_owned(), (0, 0)),
        ])
        .await;
    let started = Instant::now();
    let body = br#"{"seat":"claude","skip_map":true}"#;
    let failed = trigger(&rig.app, body)
        .await
        .expect_err("idle wait expired");
    assert!(
        failed.to_string().contains("seat is not ready for input"),
        "{failed}"
    );
    assert!(
        started.elapsed() >= Duration::from_millis(70),
        "compact did not wait for idle: {:?}",
        started.elapsed()
    );
    let reported = status(&rig.app, body).await.expect("status");
    assert_eq!(reported["stage"], "failed", "{reported}");
    assert_eq!(reported["attempts"][0]["state"], "failed", "{reported}");
    assert!(
        reported["attempts"][0]["attempt_id"]
            .as_str()
            .unwrap_or("")
            .ends_with("-1"),
        "{reported}"
    );
    rig.mux.set_capture(ready).await;
    let second = trigger(&rig.app, body)
        .await
        .expect("compact after the failed idle wait");
    assert_eq!(second["compact_sent"], true, "{second}");
    let guard = rig.app.compaction.lock().await;
    assert!(
        guard
            .book
            .attempts()
            .iter()
            .any(|attempt| attempt.id.as_str().ends_with("-2")),
        "{:?}",
        guard.book.attempts()
    );
    drop(guard);
    drop(std::fs::remove_dir_all(&rig.root));
}

#[tokio::test]
async fn cancelled_preps_keep_contiguous_attempt_numbers() {
    let rig = boot("numbers", Duration::from_millis(800)).await;
    watch(Arc::clone(&rig.mux), rig.transcript.clone());
    let body = br#"{"seat":"claude"}"#;
    for n in 1..=3 {
        let value = trigger(&rig.app, body).await.expect("prep");
        assert_eq!(value["compact_sent"], false, "{value}");
        cancel(&rig.app, body).await.expect("cancel");
        let reported = status(&rig.app, body).await.expect("status");
        let attempts = reported["attempts"].as_array().expect("attempts");
        assert_eq!(attempts.len(), n, "{reported}");
        let row = attempts.get(n.saturating_sub(1)).expect("row");
        let id = row["attempt_id"].as_str().unwrap_or("");
        assert!(id.ends_with(&format!("-{n}")), "{id}");
        assert_eq!(row["state"], "cancelled", "{reported}");
    }
    drop(std::fs::remove_dir_all(&rig.root));
}

#[tokio::test]
async fn a_faint_suggestion_is_ready_for_skip_map_compact() {
    let rig = boot("faint-compact", Duration::from_millis(800)).await;
    watch(Arc::clone(&rig.mux), rig.transcript.clone());
    rig.mux
        .set_capture(Capture::new(faint_suggestion(), (0, 0)))
        .await;
    let value = trigger(&rig.app, br#"{"seat":"claude","skip_map":true}"#)
        .await
        .expect("faint composer is idle");
    assert_eq!(value["compact_sent"], true, "{value}");
    drop(std::fs::remove_dir_all(&rig.root));
}

#[tokio::test]
async fn a_refused_compact_submit_fails_the_attempt() {
    let rig = boot("compact-refuse", Duration::from_millis(200)).await;
    watch(Arc::clone(&rig.mux), rig.transcript.clone());
    let rule = "─".repeat(40);
    let ready = Capture::new(format!("{rule}\n❯ \n{rule}\n  ? for shortcuts\n"), (0, 0));
    let seat = crate::query::find_seat(&rig.app, "claude")
        .await
        .expect("seat");
    {
        let mut guard = rig.app.compaction.lock().await;
        let id = guard.book.next_manual_id(&seat.id.to_string());
        guard.set_prep(
            "claude",
            PreparationState {
                active: true,
                attempt_id: id.0,
                mode: PrepMode::Manual,
                status: PrepStatus::Waiting,
                map_complete: true,
                paste_accepted: true,
                deadline_ms: Some(crate::state::now_ms().saturating_add(5_000)),
                reason: None,
            },
        );
    }
    rig.mux
        .script_captures(vec![
            ready.clone(),
            ready.clone(),
            Capture::new("booting\n".to_owned(), (0, 0)),
        ])
        .await;
    let body = br#"{"seat":"claude","skip_map":true}"#;
    let failed = trigger(&rig.app, body)
        .await
        .expect_err("compact submit refused");
    assert!(
        failed.to_string().contains("seat is not ready for input"),
        "{failed}"
    );
    let reported = status(&rig.app, body).await.expect("status");
    assert_eq!(reported["stage"], "failed", "{reported}");
    assert_eq!(reported["attempts"][0]["state"], "failed", "{reported}");
    rig.mux.set_capture(ready).await;
    let second = trigger(&rig.app, body)
        .await
        .expect("compact after the refused submit");
    assert_eq!(second["compact_sent"], true, "{second}");
    assert!(
        second["attempts"]
            .as_array()
            .and_then(|rows| rows.last())
            .and_then(|row| row["attempt_id"].as_str())
            .unwrap_or("")
            .ends_with("-2"),
        "{second}"
    );
    drop(std::fs::remove_dir_all(&rig.root));
}

#[tokio::test]
async fn an_unused_mint_is_reused_on_the_next_compact() {
    let rig = boot("reuse-mint", Duration::from_millis(800)).await;
    watch(Arc::clone(&rig.mux), rig.transcript.clone());
    {
        let mut guard = rig.app.compaction.lock().await;
        guard.set_prep(
            "claude",
            PreparationState {
                active: true,
                attempt_id: String::new(),
                mode: PrepMode::Manual,
                status: PrepStatus::Stopped,
                map_complete: false,
                paste_accepted: false,
                deadline_ms: None,
                reason: None,
            },
        );
    }
    let skipped = trigger(&rig.app, br#"{"seat":"claude"}"#)
        .await
        .expect_err("stopped prep");
    assert!(
        skipped.to_string().contains("preparation_stopped"),
        "{skipped}"
    );
    let value = trigger(&rig.app, br#"{"seat":"claude"}"#)
        .await
        .expect("prep reuses the released id");
    assert_eq!(value["compact_sent"], false, "{value}");
    let id = value["attempts"][0]["attempt_id"].as_str().unwrap_or("");
    assert!(id.ends_with("-1"), "{value}");
    drop(std::fs::remove_dir_all(&rig.root));
}

#[tokio::test]
async fn skip_map_compact_does_not_wait_for_a_user_turn() {
    let rig = boot("compact-wait", Duration::from_secs(3)).await;
    watch(Arc::clone(&rig.mux), rig.transcript.clone());
    let started = Instant::now();
    let value = trigger(&rig.app, br#"{"seat":"claude","skip_map":true}"#)
        .await
        .expect("compact");
    assert_eq!(value["compact_sent"], true, "{value}");
    assert!(
        started.elapsed() < Duration::from_millis(1_500),
        "compact waited for a user turn: {:?}",
        started.elapsed()
    );
    drop(std::fs::remove_dir_all(&rig.root));
}

fn faint_suggestion() -> String {
    let rule = "─".repeat(40);
    format!(
        "{rule}\n\u{1b}[2m❯ allow reading the marker path\u{1b}[22m\n{rule}\n  ? for shortcuts\n"
    )
}

fn precompact(session: &str) -> HookEvent {
    HookEvent::PreCompact {
        session_id: session.to_owned(),
        trigger: CompactTrigger::Manual,
        custom_instructions: None,
        transcript_path: PathBuf::new(),
        cwd: PathBuf::new(),
    }
}

fn compact_start(session: &str) -> HookEvent {
    HookEvent::SessionStart {
        session_id: session.to_owned(),
        source: SessionSource::Compact,
        context_tokens: None,
    }
}

fn post_compact(session: &str) -> HookEvent {
    HookEvent::PostCompact {
        session_id: session.to_owned(),
        trigger: CompactTrigger::Manual,
        compact_summary: "sum".to_owned(),
    }
}

fn stop_hook(session: &str) -> String {
    serde_json::json!({
        "hook_event_name": "Stop",
        "session_id": session
    })
    .to_string()
}

struct Boot {
    app: App,
    mux: Arc<FakeMux>,
    root: PathBuf,
    transcript: PathBuf,
}

async fn boot(label: &str, wait: Duration) -> Boot {
    let root = std::env::temp_dir().join(format!("rig-settle-{label}-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&root));
    let cwd = root.join("work");
    std::fs::create_dir_all(&cwd).expect("work");
    let home = root.join("home");
    let seats = root.join("seats");
    let mux = Arc::new(FakeMux::new());
    let rule = "─".repeat(40);
    mux.set_capture(Capture::new(
        format!("{rule}\n❯ \n{rule}\n  ? for shortcuts\n"),
        (0, 0),
    ))
    .await;
    let shared = share(&mux);
    let adapter =
        ClaudeAdapter::new(Arc::clone(&shared), seats.clone(), home.clone()).with_submit_wait(wait);
    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(adapter));
    let store = Arc::new(crewmux_store::Store::open(Path::new(":memory:")).expect("db"));
    let app = App::assemble(
        Arc::clone(&store),
        shared,
        Arc::new(registry),
        None,
        CancellationToken::new(),
        root.clone(),
    );
    let seat = SeatId::new();
    let rig = RigId::new();
    let session = "11111111-1111-4111-8111-111111111111";
    let seat_dir = seats.join(seat.to_string());
    std::fs::create_dir_all(&seat_dir).expect("seat dir");
    std::fs::write(seat_dir.join("claude-session-id"), session).expect("id");
    seed(&store, rig, seat, cwd.clone()).await;
    super::remember_launched_claude(&app, &mux, session)
        .await
        .expect("lineage");
    let transcript = crewmux_transcript::transcript_path(&home, &cwd, session).expect("path");
    Boot {
        app,
        mux,
        root,
        transcript,
    }
}

fn watch(mux: Arc<FakeMux>, path: PathBuf) {
    watch_after(mux, path, String::new());
}

/// Append the first paste with a stray suffix, then stop.
fn watch_once_contaminated(mux: Arc<FakeMux>, path: PathBuf) {
    tokio::spawn(async move {
        for _ in 0..2_000 {
            if let Some(text) = mux.last_paste().await
                && !text.is_empty()
                && !text.starts_with("/compact")
            {
                let dirty = format!("{text}\n\nx");
                append_turn(&path, &dirty).await;
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    });
}

/// Append later pastes. `seen` is a paste that already happened and must not count.
fn watch_after(mux: Arc<FakeMux>, path: PathBuf, seen: String) {
    tokio::spawn(async move {
        let mut seen = seen;
        for _ in 0..2_000 {
            if let Some(text) = mux.last_paste().await
                && text != seen
                && !text.starts_with("/compact")
            {
                seen = text.clone();
                append_turn(&path, &text).await;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    });
}

async fn append_turn(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.expect("projects");
    }
    let mut line = serde_json::json!({
        "type": "user",
        "message": {"role": "user", "content": text}
    })
    .to_string();
    line.push('\n');
    let previous = tokio::fs::read(path).await.unwrap_or_default();
    let mut body = previous;
    body.extend(line.into_bytes());
    tokio::fs::write(path, body).await.expect("jsonl");
}

fn share(mux: &Arc<FakeMux>) -> Arc<dyn Multiplexer> {
    let cloned: Arc<FakeMux> = Arc::clone(mux);
    cloned
}

async fn seed(store: &Arc<crewmux_store::Store>, rig: RigId, seat: SeatId, cwd: PathBuf) {
    write(store, move |db| {
        RigRepo::upsert(
            db,
            &Rig {
                id: rig,
                name: RigName::new("demo").expect("rig"),
                created_at: Timestamp::from_millis(0),
            },
        )?;
        SeatRepo::upsert(
            db,
            &Seat {
                id: seat,
                rig_id: rig,
                name: SeatName::new("claude").expect("seat"),
                runtime: RuntimeKind::ClaudeCode,
                cwd: Some(cwd),
                model: None,
                state: SeatState::Ready,
                reason: None,
                tmux_session: Some("seat__rig".to_owned()),
                pane_id: None,
            },
        )?;
        Ok(())
    })
    .await
    .expect("seed");
}
