//! PE-013: Enter only after the cursor is verified on the requested choice.
#![expect(clippy::panic, reason = "a bad fixture path should fail the test")]

use std::path::PathBuf;
use std::sync::Arc;

use crewmux_adapters::{
    AgentAdapter, ClaudeAdapter, DialogDriver, DialogOption, LaunchOpts, MuxDialogDriver, Readiness,
};
use crewmux_mux::{FakeMux, Multiplexer, PaneRef};
use crewmux_types::{KnownDialog, ReadinessCode, SeatId};

#[path = "common/mod.rs"]
mod support;

const UPDATE_NOW: &str = "\
Update available 0.162.0 → 0.162.1

› 1. Update now (runs brew upgrade --cask codex)
  2. Skip
";

const UPDATE_SKIP: &str = "\
Update available 0.162.0 → 0.162.1

  1. Update now (runs brew upgrade --cask codex)
› 2. Skip
";

#[tokio::test]
async fn never_enter_on_default_no_exit() {
    let trust = support::read(&support::fixtures().join("2.1.286/claude-006-trust.txt"));
    let (mux, outcome) = launch_with(&trust, false).await;
    assert!(outcome.is_ok());
    let calls = mux.recorded().await;
    assert!(!support::sent_enter(&calls), "{calls:?}");
    assert!(
        !calls.iter().flatten().any(|arg| arg == "Down"),
        "{calls:?}"
    );
}

#[tokio::test]
async fn trust_not_answered_without_opt_in() {
    let trust = support::read(&support::fixtures().join("2.1.286/claude-006-trust.txt"));
    let adapter = adapter_with(replay(&trust).await);
    let binding = binding();
    drop(adapter.launch(&binding, opts(false)).await.expect("launch"));
    let ready = adapter.check_ready(&binding).await.expect("ready");
    assert_eq!(ready, Readiness::Gate(ReadinessCode::TrustGate));
}

#[tokio::test]
async fn codex_update_answers_skip_never_update() {
    let mux = Arc::new(FakeMux::new());
    let ready = support::read(&support::fixtures().join("2.1.286/claude-020-ready.txt"));
    mux.replay_screens(vec![
        support::capture(UPDATE_NOW),
        support::capture(UPDATE_SKIP),
        support::capture(&ready),
    ])
    .await;
    let driver = MuxDialogDriver::new(support::shared(&mux), PaneRef::session("codex"));
    let choice = DialogOption {
        text: "2. Skip",
        index: Some(2),
    };
    let answered = driver
        .answer(&SeatId::new(), &KnownDialog::CodexUpdate, &choice)
        .await
        .expect("skip");
    assert_eq!(answered.choice, "2. Skip");
    let calls = mux.recorded().await;
    assert!(calls.iter().flatten().any(|arg| arg == "Down"));
    assert!(support::sent_enter(&calls));
    assert!(!calls.iter().flatten().any(|arg| arg.contains("Update now")));

    let stuck = Arc::new(FakeMux::new());
    stuck
        .replay_screens(vec![
            support::capture(UPDATE_NOW),
            support::capture(UPDATE_NOW),
        ])
        .await;
    let driver = MuxDialogDriver::new(support::shared(&stuck), PaneRef::session("codex"));
    let err = driver
        .answer(&SeatId::new(), &KnownDialog::CodexUpdate, &choice)
        .await
        .expect_err("cursor");
    assert!(err.reason.contains("cursor"));
    assert!(!support::sent_enter(&stuck.recorded().await));
}

#[tokio::test]
async fn bypass_always_escalates() {
    let text = support::read(&support::fixtures().join("2.1.286/claude-050-bypass.txt"));
    let mux = replay(&text).await;
    let adapter = adapter_with(Arc::clone(&mux));
    drop(
        adapter
            .launch(&binding(), opts(true))
            .await
            .expect("launch"),
    );
    assert!(!support::sent_enter(&mux.recorded().await));
    assert_eq!(
        adapter.check_ready(&binding()).await.expect("ready"),
        Readiness::Gate(ReadinessCode::BypassConsentGate)
    );
}

#[tokio::test]
async fn unknown_menu_escalates() {
    let text = support::read(&support::fixtures().join("2.1.286/claude-synth-reworded-trust.txt"));
    let mux = replay(&text).await;
    let adapter = adapter_with(Arc::clone(&mux));
    drop(
        adapter
            .launch(&binding(), opts(true))
            .await
            .expect("launch"),
    );
    assert!(!support::sent_enter(&mux.recorded().await));
    assert_eq!(
        adapter.check_ready(&binding()).await.expect("ready"),
        Readiness::Gate(ReadinessCode::UnknownMenu)
    );
}

#[tokio::test]
async fn cursor_verification_failure_escalates() {
    let trust = support::read(&support::fixtures().join("2.1.286/claude-006-trust.txt"));
    let mux = Arc::new(FakeMux::new());
    mux.replay_screens(vec![support::capture(&trust), support::capture(&trust)])
        .await;
    let adapter = adapter_with(Arc::clone(&mux));
    drop(
        adapter
            .launch(&binding(), opts(true))
            .await
            .expect("launch"),
    );
    let calls = mux.recorded().await;
    assert!(calls.iter().flatten().any(|arg| arg == "Down"), "{calls:?}");
    assert!(!support::sent_enter(&calls), "{calls:?}");
}

#[tokio::test]
async fn no_quit_keys_on_login_menu() {
    let text = support::read(&support::fixtures().join("cl-0-theme.txt"));
    let mux = replay(&text).await;
    let adapter = adapter_with(Arc::clone(&mux));
    drop(
        adapter
            .launch(&binding(), opts(true))
            .await
            .expect("launch"),
    );
    let calls = mux.recorded().await;
    for banned in ["Enter", "q", "C-c", "Escape", "1", "2", "y", "n"] {
        assert!(
            !calls.iter().flatten().any(|arg| arg == banned),
            "{banned} in {calls:?}"
        );
    }
}

#[tokio::test]
async fn fullscreen_answers_not_now() {
    let first = support::read(&support::fixtures().join("2.1.286/claude-007-fullscreen.txt"));
    let second = first.replace(
        "❯ 1. Yes, try it\n  2. Not now",
        "  1. Yes, try it\n❯ 2. Not now",
    );
    let ready = support::read(&support::fixtures().join("2.1.286/claude-020-ready.txt"));
    let mux = Arc::new(FakeMux::new());
    mux.replay_screens(vec![
        support::capture(&first),
        support::capture(&second),
        support::capture(&ready),
    ])
    .await;
    let adapter = adapter_with(Arc::clone(&mux));
    let binding = binding();
    drop(adapter.launch(&binding, opts(false)).await.expect("launch"));
    assert!(support::sent_enter(&mux.recorded().await));
    assert_eq!(
        adapter.check_ready(&binding).await.expect("ready"),
        Readiness::Ready
    );
    let settings = std::fs::read_to_string(adapter.seat_dir(&binding.seat).join("settings.json"))
        .expect("settings");
    let lower = settings.to_ascii_lowercase();
    assert!(!lower.contains("fullscreen"), "{settings}");
    assert!(!lower.contains("alternatescreen"), "{settings}");
}

async fn launch_with(screen: &str, auto_trust: bool) -> (Arc<FakeMux>, Result<(), String>) {
    let mux = replay(screen).await;
    let adapter = adapter_with(Arc::clone(&mux));
    let result = adapter.launch(&binding(), opts(auto_trust)).await;
    let outcome = result.map(|_| ()).map_err(|err| err.to_string());
    (mux, outcome)
}

async fn replay(screen: &str) -> Arc<FakeMux> {
    let mux = Arc::new(FakeMux::new());
    mux.replay_screens(vec![support::capture(screen)]).await;
    mux
}

fn adapter_with(mux: Arc<FakeMux>) -> ClaudeAdapter {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/claude-dialog");
    let seats = root.join(format!("seats-{}", std::process::id()));
    let mux: Arc<dyn Multiplexer> = mux;
    ClaudeAdapter::new(mux, support::null_check(), seats, root.join("home"))
}

fn binding() -> crewmux_adapters::NodeBinding {
    let cwd = std::env::temp_dir();
    crewmux_adapters::binding_for(SeatId::new(), cwd, "seat__rig")
}

fn opts(auto_trust: bool) -> LaunchOpts {
    LaunchOpts {
        session_name: "seat__rig".to_owned(),
        env: std::collections::BTreeMap::new(),
        resume: None,
        fork: false,
        auto_trust,
        budget: Some(support::fast_budget()),
        claude_bin: None,
    }
}
