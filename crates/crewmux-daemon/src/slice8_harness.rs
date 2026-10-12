//! Shared `FakeMux` setup for slice 8 tests. The database is a file so backups work.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crewmux_adapters::{AdapterRegistry, StubAdapter};
use crewmux_core::{Seat, SeatRepo};
use crewmux_mux::{CaptureRange, FakeMux, Multiplexer, PaneRef};
use crewmux_store::Store;
use crewmux_types::LaunchPosture;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::deliver::{Outbound, deliver};
use crate::launch::bring_up;
use crate::state::App;

fn share_mux(mux: Arc<FakeMux>) -> Arc<dyn Multiplexer> {
    mux
}

pub(crate) struct Rig {
    pub app: App,
    pub mux: Arc<FakeMux>,
    pub home: PathBuf,
}

pub(crate) async fn rig() -> Rig {
    let home = std::env::temp_dir().join(format!(
        "rig-s8-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos())
    ));
    std::fs::create_dir_all(&home).expect("home");
    let store = Arc::new(Store::open(&home.join("state.db")).expect("db"));
    open_on(store, home).await
}

pub(crate) async fn open_on(store: Arc<Store>, home: PathBuf) -> Rig {
    let mux = Arc::new(FakeMux::with_socket("rig-rs"));
    mux.enable_stub_scripts().await;
    let shared = share_mux(Arc::clone(&mux));
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
        home.clone(),
    );
    app.launch_workers().await;
    Rig { app, mux, home }
}

pub(crate) fn spec(name: &str, seats: &[(&str, &str)]) -> String {
    let mut text = format!("name: {name}\nseats:\n");
    for (seat, cwd) in seats {
        text.push_str("  - name: ");
        text.push_str(seat);
        text.push_str("\n    runtime: stub\n    cwd: ");
        text.push_str(cwd);
        text.push('\n');
    }
    text
}

pub(crate) async fn up(app: &App, document: &str) {
    bring_up(app, document).await.expect("up");
}

pub(crate) async fn kill_sessions(app: &App) {
    let seats = crate::db::read(&app.store, |conn| conn.list_all())
        .await
        .expect("seats");
    for seat in seats {
        if let Some(name) = seat.tmux_session {
            drop(app.mux.kill_session(&name).await);
        }
    }
    crate::reconcile::reconcile(app).await.expect("reconcile");
}

pub(crate) async fn seat(app: &App, name: &str) -> Seat {
    crate::query::find_seat(app, name).await.expect(name)
}

pub(crate) fn seat_field<'a>(body: &'a Value, name: &str, field: &str) -> &'a str {
    body.get("seats")
        .and_then(Value::as_array)
        .and_then(|seats| {
            seats
                .iter()
                .find(|seat| seat.get("seat").and_then(Value::as_str) == Some(name))
        })
        .and_then(|seat| seat.get(field))
        .and_then(Value::as_str)
        .unwrap_or("")
}

pub(crate) fn never_ready(body: &Value) {
    let seats = body.get("seats").and_then(Value::as_array).expect("seats");
    assert_ne!(seats.len(), 0, "{body}");
    for seat in seats {
        let outcome = seat.get("outcome").and_then(Value::as_str).unwrap_or("");
        assert_ne!(outcome, "ready", "{seat}");
    }
    assert_ne!(body.get("outcome").and_then(Value::as_str), Some("ready"));
}

pub(crate) async fn send(app: &App, address: &str, body: &str) -> Value {
    deliver(
        app,
        Outbound {
            from: "operator@host".to_owned(),
            targets: vec![address.to_owned()],
            rig: None,
            all: false,
            body: body.to_owned(),
            interact: false,
            reason: None,
            verify: false,
            track_direct: true,
        },
    )
    .await
    .expect("send")
}

pub(crate) async fn screens(app: &App) -> String {
    let panes = app.mux.list_panes().await.expect("panes");
    let mut text = String::new();
    for pane in panes {
        let shot = app
            .mux
            .capture(&PaneRef::session(pane.session), CaptureRange::Visible)
            .await
            .expect("capture");
        text.push_str(&shot.text);
        text.push('\n');
    }
    text
}

pub(crate) async fn restore(app: &App, name: &str, posture: LaunchPosture) -> Value {
    crate::restore::restore_rig(app, name, None, posture)
        .await
        .expect("restore")
}

pub(crate) async fn shutdown(rig: Rig) -> PathBuf {
    rig.app.cancel.cancel();
    rig.app.tasks.close();
    rig.app.tasks.wait().await;
    let home = rig.home.clone();
    drop(rig);
    home
}
