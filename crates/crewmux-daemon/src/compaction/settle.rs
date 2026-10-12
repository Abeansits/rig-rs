//! A finished or cancelled manual compact does not stick the seat on `compacting`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crewmux_adapters::{AdapterRegistry, ClaudeAdapter};
use crewmux_core::{Rig, RigRepo, Seat, SeatRepo, SeatState, Timestamp};
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
