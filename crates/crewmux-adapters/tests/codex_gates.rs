//! Known Codex dialogs: answer Skip, escalate the rest.
#![expect(
    clippy::wildcard_enum_match_arm,
    reason = "the test asserts one screen and fails on every other variant"
)]

#[path = "common/mod.rs"]
mod support;

use std::fs;
use std::sync::Arc;

use crewmux_adapters::codex::{CodexScreen, classify, is_rig_tool};
use crewmux_adapters::dialog::{AnswerPolicy, DialogChoice, KnownDialog, planned_choice};
use crewmux_adapters::{AgentAdapter, Readiness};
use crewmux_mux::{Capture, FakeMux};
use crewmux_types::ReadinessCode;

use support::{
    LOGGED_IN, Motion, adapter, binding, calls_include, drive_menu, fixtures, home_with, opts,
};

#[tokio::test]
async fn background_server_menu_is_attention() {
    let text = fs::read_to_string(fixtures().join("screens/codex/0.162.0/codex-000-firstrun.txt"))
        .expect("menu");
    assert!(matches!(
        classify(&text),
        CodexScreen::Dialog(KnownDialog::CodexBackgroundServer)
    ));
    assert!(
        planned_choice(
            &KnownDialog::CodexBackgroundServer,
            AnswerPolicy::default(),
            None
        )
        .is_none()
    );
    let verdict = show(&text).await;
    assert_eq!(
        verdict.0,
        Readiness::Gate(ReadinessCode::CodexBackgroundServer)
    );
    assert!(!verdict.1);
}

#[tokio::test]
async fn mcp_tool_approval_is_attention() {
    let text =
        fs::read_to_string(fixtures().join("screens/codex/0.162.0/codex-106-s6-approval.txt"))
            .expect("approval");
    match classify(&text) {
        CodexScreen::Dialog(KnownDialog::CodexMcpToolApproval { server, tool }) => {
            assert_eq!(server, "rig");
            assert_eq!(tool, "seat_send");
            assert!(!is_rig_tool(&tool));
        }
        other => panic!("{other:?}"),
    }
    let verdict = show(&text).await;
    assert_eq!(verdict.0, Readiness::Gate(ReadinessCode::McpToolApproval));
    assert!(!verdict.1);
}

#[test]
fn agent_send_is_allow_for_this_session() {
    let dialog = KnownDialog::CodexMcpToolApproval {
        server: "rig".to_owned(),
        tool: "agent_send".to_owned(),
    };
    assert_eq!(
        planned_choice(&dialog, AnswerPolicy::default(), None),
        Some(DialogChoice::AllowForSession)
    );
    let spike = KnownDialog::CodexMcpToolApproval {
        server: "rig".to_owned(),
        tool: "seat_send".to_owned(),
    };
    assert!(planned_choice(&spike, AnswerPolicy::default(), None).is_none());
    let off = AnswerPolicy {
        auto_trust: false,
        auto_approve_rig_tools: false,
    };
    assert!(planned_choice(&dialog, off, None).is_none());
}

#[tokio::test]
async fn update_answers_skip_never_update() {
    let report = drive_menu(
        "Update available! 0.162.0 → 0.162.1",
        &[
            "Update now (runs brew upgrade --cask codex)",
            "Skip",
            "Skip until next version",
        ],
        DialogChoice::Skip,
        Motion::Follow,
    )
    .await;
    assert_eq!(
        report.result,
        crewmux_adapters::dialog::DriveResult::Answered
    );
    assert_eq!(report.keys.first().map(KeyName::of), Some("down"));
    assert_eq!(report.keys.last().map(KeyName::of), Some("enter"));
    let before = report.before_enter.expect("enter");
    let selected = crewmux_adapters::dialog::menu_options(&before)
        .into_iter()
        .find(|opt| opt.selected)
        .expect("cursor");
    assert_eq!(selected.label, "Skip");
    assert!(
        report
            .keys
            .iter()
            .all(|key| matches!(KeyName::of(key), "up" | "down" | "enter"))
    );
}

struct KeyName;

impl KeyName {
    fn of(key: &crewmux_mux::Key) -> &'static str {
        match key {
            crewmux_mux::Key::Up => "up",
            crewmux_mux::Key::Down => "down",
            crewmux_mux::Key::Enter => "enter",
            crewmux_mux::Key::Escape => "escape",
            crewmux_mux::Key::CtrlC => "ctrl-c",
            crewmux_mux::Key::Tab => "tab",
            crewmux_mux::Key::Literal(_) => "literal",
        }
    }
}

#[expect(
    clippy::expect_used,
    reason = "a failed launch in the gate fixture fails the test"
)]
async fn show(text: &str) -> (Readiness, bool) {
    let home = home_with(LOGGED_IN);
    let mux = Arc::new(FakeMux::new());
    let adapter = adapter(&mux, &home.bin);
    let binding = binding(&home.cwd, "seat");
    adapter
        .launch(&binding, opts(&home, "seat"))
        .await
        .expect("launch");
    let before = mux.recorded().await.len();
    mux.set_capture(Capture::new(text.to_owned(), (0, 0))).await;
    let verdict = adapter.check_ready(&binding).await.expect("ready");
    let calls = mux.recorded().await;
    let extra = calls.get(before..).unwrap_or(&[]);
    (verdict, calls_include(extra, "send-keys"))
}
