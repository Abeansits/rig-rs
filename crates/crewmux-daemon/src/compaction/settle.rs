//! A finished or cancelled manual compact does not stick the seat on `compacting`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crewmux_adapters::{AdapterRegistry, ClaudeAdapter};
use crewmux_core::{Rig, RigRepo, Seat, SeatRepo, SeatState, Timestamp};
use crewmux_health::compaction::{
    AttemptState, CompactTrigger, HookEvent, ManualCompactionStage, SessionSource,
};
use crewmux_mux::{Capture, FakeMux, Multiplexer};
use crewmux_types::{RigId, RigName, RuntimeKind, SeatId, SeatName};
use tokio_util::sync::CancellationToken;

use super::trigger::{cancel, release_finished, trigger};
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
async fn compact_after_send_failed_then_incomplete_sends_again() {
    let rig = boot("failed-prep", Duration::from_millis(800)).await;
    watch(Arc::clone(&rig.mux), rig.transcript.clone());
    rig.mux
        .set_capture(Capture::new("booting\n".to_owned(), (0, 0)))
        .await;
    let body = br#"{"seat":"claude"}"#;
    let first = trigger(&rig.app, body).await.expect("send while not ready");
    assert_eq!(first["reason"], "send_failed", "{first}");
    {
        let mut guard = rig.app.compaction.lock().await;
        let prep = guard.preps.get_mut("claude").expect("prep");
        assert!(prep.active);
        prep.deadline_ms = Some(0);
    }
    let failed = trigger(&rig.app, body).await;
    let message = failed.expect_err("deadline").to_string();
    assert!(message.contains("preparation_incomplete"), "{message}");
    let rule = "─".repeat(40);
    rig.mux
        .set_capture(Capture::new(
            format!("{rule}\n❯ \n{rule}\n  ? for shortcuts\n"),
            (0, 0),
        ))
        .await;
    let second = trigger(&rig.app, br#"{"seat":"claude","skip_map":true}"#)
        .await
        .expect("compact after a failed prep");
    assert_eq!(second["compact_sent"], true, "{second}");
    assert!(
        !second.to_string().contains("preparation_stopped"),
        "{second}"
    );
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
    tokio::spawn(async move {
        let mut seen = String::new();
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
