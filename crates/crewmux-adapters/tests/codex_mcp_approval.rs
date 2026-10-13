//! AC11–AC12: the real Codex approval capture, with the tool renamed in memory.

use std::fs;
use std::sync::Arc;

use crewmux_adapters::dialog::{AnswerPolicy, DialogDriver, DriveOutcome, MuxPane, SeatContext};
use crewmux_adapters::{CODEX_DIALOGS, EscalateReason};
use crewmux_mux::{Capture, FakeMux, Multiplexer, PaneRef};
use crewmux_types::{DialogId, ReadinessCode};

#[path = "codex_support.rs"]
mod support;

#[tokio::test]
async fn default_policy_escalates_agent_send_without_keys() {
    let text = renamed();
    assert!(text.contains("agent_send"));
    assert!(!text.contains("seat_send"));
    let (mux, outcome) = drive(&text, false, &[]).await;
    assert_eq!(
        outcome,
        DriveOutcome::Escalated {
            id: Some(DialogId::CodexMcpToolApproval),
            reason: EscalateReason::Policy,
        }
    );
    assert_eq!(
        crewmux_adapters::codex::classify(&text),
        crewmux_adapters::codex::CodexScreen::Dialog(DialogId::CodexMcpToolApproval)
    );
    assert!(matches!(
        ReadinessCode::McpToolApproval,
        ReadinessCode::McpToolApproval
    ));
    assert_eq!(mux.recorded().await, Vec::<Vec<String>>::new());
}

#[tokio::test]
async fn enabled_gate_selects_allow_for_this_session() {
    let text = renamed();
    let moved = cursor_on_session(&text);
    assert!(moved.contains("› 2. Allow for this session"), "{moved}");
    let ready = "╭────────────────╮\n│ ›              │\n╰────────────────╯\n  ? for shortcuts\n";
    let (mux, outcome) = drive(&text, true, &[&moved, ready]).await;
    assert_eq!(
        outcome,
        DriveOutcome::Answered {
            id: DialogId::CodexMcpToolApproval,
            label: "Allow for this session",
        }
    );
    let calls = mux.recorded().await;
    assert!(calls.iter().flatten().any(|arg| arg == "Down"), "{calls:?}");
    assert!(
        calls.iter().flatten().any(|arg| arg == "Enter"),
        "{calls:?}"
    );
    assert!(!calls.iter().flatten().any(|arg| arg == "Allow"));
}

#[tokio::test]
async fn foreign_server_and_seat_send_do_not_enter() {
    let foreign = renamed().replace("Allow the rig MCP server", "Allow the other MCP server");
    let (mux, outcome) = drive(&foreign, true, &[]).await;
    assert!(matches!(
        outcome,
        DriveOutcome::Escalated {
            reason: EscalateReason::Policy,
            ..
        }
    ));
    assert_eq!(mux.recorded().await, Vec::<Vec<String>>::new());

    let spike = fs::read_to_string(fixture()).expect("fixture");
    assert!(spike.contains("seat_send"));
    let (mux, outcome) = drive(&spike, true, &[]).await;
    assert!(matches!(
        outcome,
        DriveOutcome::Escalated {
            reason: EscalateReason::Policy,
            ..
        }
    ));
    assert_eq!(mux.recorded().await, Vec::<Vec<String>>::new());
}

#[expect(
    clippy::expect_used,
    reason = "a missing approval fixture fails the test"
)]
fn renamed() -> String {
    fs::read_to_string(fixture())
        .expect("fixture")
        .replace("seat_send", "agent_send")
}

fn fixture() -> std::path::PathBuf {
    support::fixtures().join("0.162.0/codex-106-s6-approval.txt")
}

fn cursor_on_session(text: &str) -> String {
    text.replace(
        "  › 1. Allow                   Run the tool and continue\n    2. Allow for this session  Run the tool and remember this choice for this session",
        "    1. Allow                   Run the tool and continue\n  › 2. Allow for this session  Run the tool and remember this choice for this session",
    )
}

fn capture(text: &str) -> Capture {
    Capture::new(text.to_owned(), (0, 0))
}

#[expect(clippy::expect_used, reason = "a driver error fails the approval test")]
async fn drive(first: &str, approve: bool, rest: &[&str]) -> (Arc<FakeMux>, DriveOutcome) {
    let mux = Arc::new(FakeMux::new());
    let mut frames = vec![capture(first)];
    for frame in rest {
        frames.push(capture(frame));
    }
    mux.replay_screens(frames).await;
    let cwd = std::env::temp_dir();
    let seat = SeatContext {
        cwd: &cwd,
        home: &cwd,
    };
    let io = MuxPane::new(shared(&mux), PaneRef::session("codex"));
    let outcome = DialogDriver::default()
        .drive(&io, CODEX_DIALOGS, policy(approve), &seat)
        .await
        .expect("drive");
    (mux, outcome)
}

fn policy(auto_approve_rig_tools: bool) -> AnswerPolicy {
    AnswerPolicy {
        auto_trust: false,
        auto_approve_rig_tools,
    }
}

fn shared(mux: &Arc<FakeMux>) -> Arc<dyn Multiplexer> {
    let cloned: Arc<FakeMux> = Arc::clone(mux);
    cloned
}
