//! `POST /api/seats/submit` calls [`crewmux_adapters::ClaudeAdapter`]'s submit.

use crewmux_core::Seat;
use crewmux_mux::PaneRef;
use serde_json::{Value, json};

use crate::error::Error;
use crate::query::find_seat;
use crate::state::App;

pub(crate) async fn submit_text(app: &App, name: &str, text: &str) -> Result<Value, Error> {
    let seat = find_seat(app, name).await?;
    let binding = binding_for(&seat)?;
    let adapter = app
        .registry
        .get(seat.runtime)
        .ok_or_else(|| Error::Message(format!("no adapter for {}", seat.runtime.as_str())))?;
    let receipt = adapter.submit(&binding, text).await?;
    Ok(json!({"seat": name, "bytes": receipt.bytes}))
}

fn binding_for(seat: &Seat) -> Result<crewmux_adapters::NodeBinding, Error> {
    let Some(session) = seat.tmux_session.clone() else {
        return Err(Error::Message(format!(
            "seat {} has no session",
            seat.name.as_str()
        )));
    };
    let cwd = seat.cwd.clone().unwrap_or_default();
    let mut binding = crewmux_adapters::binding_for(seat.id, cwd, &session);
    binding.pane = PaneRef {
        session,
        window: Some(0),
        pane: Some(0),
        pane_id: seat.pane_id.clone(),
    };
    Ok(binding)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use crewmux_adapters::{AdapterRegistry, ClaudeAdapter};
    use crewmux_core::{Rig, RigRepo, Seat, SeatRepo, SeatState, Timestamp};
    use crewmux_mux::{Capture, FakeMux, Multiplexer};
    use crewmux_types::{RigId, RigName, RuntimeKind, SeatId, SeatName};
    use tokio_util::sync::CancellationToken;

    use super::submit_text;
    use crate::db::write;
    use crate::state::App;

    #[tokio::test]
    async fn submit_text_reaches_claude_io() {
        let root = std::env::temp_dir().join(format!("rig-daemon-submit-{}", std::process::id()));
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
        let adapter = ClaudeAdapter::new(Arc::clone(&shared), seats.clone(), home.clone())
            .with_submit_wait(Duration::from_secs(2));
        let mut registry = AdapterRegistry::new();
        registry.register(Arc::new(adapter));
        let store =
            Arc::new(crewmux_store::Store::open(std::path::Path::new(":memory:")).expect("db"));
        let app = App::new(
            Arc::clone(&store),
            shared,
            None,
            CancellationToken::new(),
            Arc::new(registry),
        );
        let seat = SeatId::new();
        let rig = RigId::new();
        let cwd = root.join("work");
        let session = "11111111-1111-4111-8111-111111111111";
        let seat_dir = seats.join(seat.to_string());
        std::fs::create_dir_all(&seat_dir).expect("seat dir");
        std::fs::write(seat_dir.join("claude-session-id"), session).expect("id");
        let path = crewmux_transcript::transcript_path(&home, &cwd, session).expect("path");
        watch_transcript(Arc::clone(&mux), path.clone());
        seed(&store, rig, seat, cwd.clone()).await;
        let value = submit_text(&app, "claude", "hello").await.expect("submit");
        assert_eq!(value["bytes"], 5);
        assert!(path.is_file(), "transcript was not written");
        assert!(
            mux.recorded()
                .await
                .iter()
                .flatten()
                .any(|arg| arg == "Enter")
        );
        drop(std::fs::remove_dir_all(&root));
    }

    fn share(mux: &Arc<FakeMux>) -> Arc<dyn Multiplexer> {
        let cloned: Arc<FakeMux> = Arc::clone(mux);
        cloned
    }

    fn watch_transcript(mux: Arc<FakeMux>, path: std::path::PathBuf) {
        tokio::spawn(async move {
            for _ in 0..200 {
                let calls = mux.recorded().await;
                if calls.iter().flatten().any(|arg| arg == "paste-buffer") {
                    if let Some(parent) = path.parent() {
                        tokio::fs::create_dir_all(parent).await.expect("projects");
                    }
                    let line = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n";
                    tokio::fs::write(&path, line).await.expect("jsonl");
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
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
}
