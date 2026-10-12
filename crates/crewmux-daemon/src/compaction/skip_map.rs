//! `/compact` still counts as sent when the transcript skips that record.

use std::sync::Arc;
use std::time::Duration;

use crewmux_adapters::{AdapterRegistry, ClaudeAdapter};
use crewmux_core::{Rig, RigRepo, Seat, SeatRepo, SeatState, Timestamp};
use crewmux_health::compaction::{PrepMode, PrepStatus, PreparationState};
use crewmux_mux::{Capture, FakeMux, Multiplexer};
use crewmux_types::{RigId, RigName, RuntimeKind, SeatId, SeatName};
use tokio_util::sync::CancellationToken;

use super::trigger::trigger;
use crate::db::write;
use crate::state::App;

#[tokio::test]
async fn unverified_compact_counts_as_sent() {
    let root = std::env::temp_dir().join(format!("rig-skip-map-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&root));
    std::fs::create_dir_all(root.join("work")).expect("work");
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
    let adapter = ClaudeAdapter::new(Arc::clone(&shared), seats.clone(), home)
        .with_submit_wait(Duration::from_millis(40));
    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(adapter));
    let store = Arc::new(crewmux_store::Store::open(std::path::Path::new(":memory:")).expect("db"));
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
    let cwd = root.join("work");
    let session = "11111111-1111-4111-8111-111111111111";
    let seat_dir = seats.join(seat.to_string());
    std::fs::create_dir_all(&seat_dir).expect("seat dir");
    std::fs::write(seat_dir.join("claude-session-id"), session).expect("id");
    seed(&store, rig, seat, cwd).await;
    {
        let mut guard = app.compaction.lock().await;
        guard.set_prep(
            "claude",
            PreparationState {
                active: true,
                attempt_id: "attempt-skip".to_owned(),
                mode: PrepMode::Manual,
                status: PrepStatus::Waiting,
                map_complete: true,
                deadline_ms: Some(u64::MAX),
                reason: None,
            },
        );
    }
    let value = trigger(&app, br#"{"seat":"claude","skip_map":true}"#)
        .await
        .expect("trigger");
    assert_eq!(value["compact_sent"], true);
    assert!(
        !value
            .to_string()
            .contains("occupant_generation_unavailable"),
        "{value}"
    );
    let enters = mux
        .recorded()
        .await
        .iter()
        .flatten()
        .filter(|arg| arg.as_str() == "Enter")
        .count();
    assert_eq!(enters, 1, "only /compact is submitted");
    drop(std::fs::remove_dir_all(&root));
}

fn share(mux: &Arc<FakeMux>) -> Arc<dyn Multiplexer> {
    let cloned: Arc<FakeMux> = Arc::clone(mux);
    cloned
}

async fn seed(
    store: &Arc<crewmux_store::Store>,
    rig: RigId,
    seat: SeatId,
    cwd: std::path::PathBuf,
) {
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
