//! `rig send` classifies a Claude pane with the Claude screen parser (issue #34).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crewmux_adapters::{AdapterRegistry, ClaudeAdapter, Readiness};
use crewmux_core::{Rig, RigRepo, Seat, SeatRepo, SeatState, Timestamp};
use crewmux_mux::{Capture, FakeMux, Multiplexer, SessionSpec};
use crewmux_types::{ReadinessCode, RigId, RigName, RuntimeKind, SeatId, SeatName};
use tokio_util::sync::CancellationToken;

use crate::db::write;
use crate::deliver::{Outbound, deliver};
use crate::state::App;

#[tokio::test]
async fn claude_ready_panes_are_not_refused_as_unrecognised() {
    for name in [
        "claude-2.1.287-ready-after-trust.txt",
        "claude-2.1.287-ready-idle-auto-mode-update-banner.txt",
    ] {
        let text = fixture(name);
        assert_eq!(
            crewmux_adapters::classify_runtime(RuntimeKind::ClaudeCode, &text),
            Readiness::Ready,
            "{name}"
        );
        let rig = boot(name, &text).await;
        let before = paste_count(&rig.mux).await;
        let sent = deliver(&rig.app, outbound("ping")).await;
        let shown = match &sent {
            Ok(value) => value.to_string(),
            Err(err) => err.to_string(),
        };
        assert!(!shown.contains("unrecognised_screen"), "{name} {shown}");
        assert!(
            paste_count(&rig.mux).await > before,
            "{name} never pasted: {shown}"
        );
        drop(std::fs::remove_dir_all(&rig.root));
    }
}

#[tokio::test]
async fn a_stub_prompt_on_a_claude_seat_is_not_ready() {
    let text = "stub> \n";
    assert_ne!(
        crewmux_adapters::classify_runtime(RuntimeKind::ClaudeCode, text),
        Readiness::Ready
    );
    assert_eq!(
        crewmux_adapters::classify_runtime(RuntimeKind::Stub, text),
        Readiness::Ready
    );
    let rig = boot("stub-on-claude", text).await;
    let before = paste_count(&rig.mux).await;
    let sent = deliver(&rig.app, outbound("ping"))
        .await
        .expect_err("stub pane");
    let message = sent.to_string();
    assert!(
        message.contains(ReadinessCode::UnrecognisedScreen.as_str()),
        "{message}"
    );
    assert_eq!(paste_count(&rig.mux).await, before, "{message}");
    drop(std::fs::remove_dir_all(&rig.root));
}

#[tokio::test]
async fn ps_reports_attention_while_a_permission_dialog_is_up() {
    let ready = fixture("claude-2.1.287-ready-after-trust.txt");
    let rig = boot("ps-dialog", &ready).await;
    let dialog = fixture("claude-2.1.287-permission-read-outside.txt");
    rig.mux.set_capture(Capture::new(dialog, (0, 0))).await;
    let listed = crate::query::list_seats(&rig.app).await.expect("ps");
    assert_eq!(seat_field(&listed, "state"), "attention");
    assert_eq!(seat_field(&listed, "code"), "unknown_menu");
    let stored = crate::query::find_seat(&rig.app, "claude")
        .await
        .expect("seat");
    assert!(matches!(stored.state, SeatState::Ready));
    drop(std::fs::remove_dir_all(&rig.root));
}

#[tokio::test]
async fn a_failed_send_clears_each_typed_envelope_line() {
    let rule = "─".repeat(40);
    let screen = format!("{rule}\n❯ From: operator@host\nTo: cl@pr18\n---\n{rule}\n");
    let rig = boot("residue", &screen).await;
    let pane = crewmux_mux::PaneRef::session("seat__rig");
    let cleared = crate::paste_check::clear_composer(
        &rig.app,
        &pane,
        crewmux_mux::CaptureRange::Visible,
        3,
        RuntimeKind::ClaudeCode,
    )
    .await
    .expect("clear");
    assert!(cleared);
    let shot = rig
        .mux
        .capture(&pane, crewmux_mux::CaptureRange::Visible)
        .await
        .expect("capture");
    assert!(
        crewmux_adapters::claude::classify_text(&shot.text).is_ready(),
        "{}",
        shot.text
    );
    assert!(!shot.text.contains("From:"), "{}", shot.text);
    assert!(!shot.text.contains("To:"), "{}", shot.text);
    drop(std::fs::remove_dir_all(&rig.root));
}

#[tokio::test]
async fn a_faint_suggestion_is_delivered() {
    let rig = boot("faint-send", &faint_suggestion()).await;
    let before = paste_count(&rig.mux).await;
    let sent = deliver(&rig.app, outbound("ping")).await;
    let shown = match &sent {
        Ok(value) => value.to_string(),
        Err(err) => err.to_string(),
    };
    assert!(
        paste_count(&rig.mux).await > before,
        "plain capture treated the suggestion as a draft: {shown}"
    );
    drop(std::fs::remove_dir_all(&rig.root));
}

#[tokio::test]
async fn clear_composer_keeps_a_faint_suggestion_from_reading_as_a_draft() {
    let screen = faint_suggestion();
    let rig = boot("faint-clear", &screen).await;
    let pane = crewmux_mux::PaneRef::session("seat__rig");
    rig.mux
        .replay_screens(vec![Capture::new(screen, (0, 0))])
        .await;
    let cleared = crate::paste_check::clear_composer(
        &rig.app,
        &pane,
        crewmux_mux::CaptureRange::Visible,
        1,
        RuntimeKind::ClaudeCode,
    )
    .await
    .expect("clear");
    assert!(cleared, "paste check dropped the escapes and saw a draft");
    drop(std::fs::remove_dir_all(&rig.root));
}

fn faint_suggestion() -> String {
    let rule = "─".repeat(40);
    format!("{rule}\n\u{1b}[2m❯ allow reading the marker path\u{1b}[22m\n{rule}\n")
}

fn seat_field(body: &serde_json::Value, field: &str) -> String {
    body.get("rigs")
        .and_then(|rigs| rigs.get(0))
        .and_then(|rig| rig.get("seats"))
        .and_then(|seats| seats.get(0))
        .and_then(|seat| seat.get(field))
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_owned()
}

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../crewmux-adapters/tests/fixtures/screens/claude/2.1.287")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

fn outbound(body: &str) -> Outbound {
    Outbound {
        from: "operator@host".to_owned(),
        targets: vec!["claude".to_owned()],
        rig: None,
        all: false,
        body: body.to_owned(),
        interact: false,
        reason: None,
        verify: false,
        track_direct: false,
    }
}

struct Boot {
    app: App,
    mux: Arc<FakeMux>,
    root: PathBuf,
}

async fn boot(label: &str, screen: &str) -> Boot {
    let root = std::env::temp_dir().join(format!(
        "rig-claude-send-{}-{}",
        label.replace('.', "-"),
        std::process::id()
    ));
    drop(std::fs::remove_dir_all(&root));
    let cwd = root.join("work");
    std::fs::create_dir_all(&cwd).expect("work");
    let home = root.join("home");
    let seats = root.join("seats");
    let mux = Arc::new(FakeMux::new());
    mux.set_capture(Capture::new(screen.to_owned(), (0, 0)))
        .await;
    let shared = share(&mux);
    let adapter = ClaudeAdapter::new(Arc::clone(&shared), seats.clone(), home.clone());
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
    let session = "11111111-1111-4111-8111-111111111111";
    let seat_dir = seats.join(seat.to_string());
    std::fs::create_dir_all(&seat_dir).expect("seat dir");
    std::fs::write(seat_dir.join("claude-session-id"), session).expect("id");
    seed(&store, rig, seat, cwd.clone()).await;
    let spec = SessionSpec::new("seat__rig", cwd);
    let pane = mux.create_session(&spec).await.expect("pane");
    mux.set_pane_process(
        &pane.session,
        Some(1),
        "claude",
        Some(&format!("claude --session-id {session}")),
    )
    .await;
    let loaded = crate::query::find_seat(&app, "claude").await.expect("seat");
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
