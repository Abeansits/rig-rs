//! A coloured Codex composer delivers when idle and does not deliver a typed draft.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crewmux_adapters::{AdapterRegistry, CodexAdapter};
use crewmux_core::{Rig, RigRepo, Seat, SeatRepo, SeatState, Timestamp};
use crewmux_mux::{Capture, FakeMux, Multiplexer, SessionSpec};
use crewmux_types::{RigId, RigName, RuntimeKind, SeatId, SeatName};
use tokio_util::sync::CancellationToken;

use crate::db::write;
use crate::deliver::{Outbound, deliver};
use crate::state::App;

#[tokio::test]
async fn a_coloured_idle_codex_screen_is_delivered() {
    let rig = boot("idle", &coloured_idle()).await;
    let before = paste_count(&rig.mux).await;
    let sent = deliver(&rig.app, outbound("ping")).await;
    let shown = match &sent {
        Ok(value) => value.to_string(),
        Err(err) => err.to_string(),
    };
    assert!(
        paste_count(&rig.mux).await > before,
        "coloured idle screen was not delivered: {shown}"
    );
    drop(std::fs::remove_dir_all(&rig.root));
}

#[tokio::test]
async fn a_coloured_typed_draft_is_not_delivered() {
    let screen = coloured_draft();
    assert!(matches!(
        crewmux_adapters::codex::classify(&screen),
        crewmux_adapters::codex::CodexScreen::Draft { .. }
    ));
    let rig = boot("draft", &screen).await;
    let before = paste_count(&rig.mux).await;
    let sent = deliver(&rig.app, outbound("ping")).await;
    assert!(sent.is_err(), "coloured draft was delivered: {sent:?}");
    assert_eq!(
        paste_count(&rig.mux).await,
        before,
        "paste ran over a coloured draft"
    );
    drop(std::fs::remove_dir_all(&rig.root));
}

fn coloured_idle() -> String {
    "\u{1b}[1m›\u{1b}[22m \u{1b}[2mAsk Codex to do anything\u{1b}[22m\n  ? for shortcuts\n"
        .to_owned()
}

fn coloured_draft() -> String {
    "\u{1b}[1m›\u{1b}[22m hello\n  ? for shortcuts\n".to_owned()
}

struct Boot {
    app: App,
    mux: Arc<FakeMux>,
    root: PathBuf,
}

async fn boot(label: &str, screen: &str) -> Boot {
    let root = std::env::temp_dir().join(format!("rig-codex-send-{label}-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&root));
    let cwd = root.join("work");
    std::fs::create_dir_all(&cwd).expect("work");
    let mux = Arc::new(FakeMux::new());
    mux.set_capture(Capture::new(screen.to_owned(), (0, 0)))
        .await;
    let shared = share(&mux);
    let adapter = CodexAdapter::new(Arc::clone(&shared));
    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(adapter));
    let store = Arc::new(crewmux_store::Store::open(Path::new(":memory:")).expect("db"));
    let mut app = App::assemble(
        Arc::clone(&store),
        shared,
        Arc::new(registry),
        None,
        CancellationToken::new(),
        root.clone(),
    );
    app.transcripts = false;
    let seat = SeatId::new();
    let rig = RigId::new();
    seed(&store, rig, seat, cwd.clone()).await;
    let spec = SessionSpec::new("seat__rig", cwd);
    let pane = mux.create_session(&spec).await.expect("pane");
    mux.set_pane_process(&pane.session, Some(1), "codex", Some("codex"))
        .await;
    let loaded = crate::query::find_seat(&app, "codex").await.expect("seat");
    crate::lineage::record(&app, &loaded, &pane)
        .await
        .expect("lineage");
    Boot { app, mux, root }
}

fn share(mux: &Arc<FakeMux>) -> Arc<dyn Multiplexer> {
    let cloned: Arc<FakeMux> = Arc::clone(mux);
    cloned
}

async fn paste_count(mux: &FakeMux) -> usize {
    mux.recorded()
        .await
        .iter()
        .filter(|argv| argv.iter().any(|arg| arg == "paste-buffer"))
        .count()
}

fn outbound(body: &str) -> Outbound {
    Outbound {
        from: "operator@host".to_owned(),
        targets: vec!["codex".to_owned()],
        rig: None,
        all: false,
        body: body.to_owned(),
        interact: false,
        reason: None,
        verify: false,
        track_direct: false,
    }
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
                name: SeatName::new("codex").expect("seat"),
                runtime: RuntimeKind::Codex,
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
