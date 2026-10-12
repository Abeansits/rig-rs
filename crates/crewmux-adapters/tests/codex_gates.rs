//! Known Codex dialogs: answer Skip, escalate the rest.

#[path = "codex_support.rs"]
mod support;

use std::fs;
use std::sync::Arc;

use crewmux_adapters::codex::{CodexScreen, classify, is_rig_tool};
use crewmux_adapters::dialog::{AnswerPolicy, Plan, SeatContext, plan, recognise};
use crewmux_adapters::{AgentAdapter, CODEX_DIALOGS, Readiness};
use crewmux_mux::{Capture, FakeMux};
use crewmux_types::{DialogId, ReadinessCode};

use support::{LOGGED_IN, adapter, binding, calls_include, fixtures, home_with, opts};

#[tokio::test]
async fn background_server_menu_is_attention() {
    let text = fs::read_to_string(fixtures().join("0.162.0/codex-000-firstrun.txt")).expect("menu");
    assert!(matches!(
        classify(&text),
        CodexScreen::Dialog(DialogId::CodexBackgroundServer)
    ));
    let found = recognise(CODEX_DIALOGS, &text).expect("background");
    let cwd = home_cwd();
    let seat = SeatContext {
        cwd: &cwd,
        home: &cwd,
    };
    assert!(matches!(
        plan(&found, approve_policy(true), &seat),
        Plan::Escalate(_)
    ));
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
        fs::read_to_string(fixtures().join("0.162.0/codex-106-s6-approval.txt")).expect("approval");
    assert!(matches!(
        classify(&text),
        CodexScreen::Dialog(DialogId::CodexMcpToolApproval)
    ));
    let found = recognise(CODEX_DIALOGS, &text).expect("mcp");
    assert_eq!(found.params.server.as_deref(), Some("rig"));
    assert_eq!(found.params.tool.as_deref(), Some("seat_send"));
    assert!(!is_rig_tool("seat_send"));
    let verdict = show(&text).await;
    assert_eq!(verdict.0, Readiness::Gate(ReadinessCode::McpToolApproval));
    assert!(!verdict.1);
}

#[test]
fn agent_send_is_allow_for_this_session() {
    let screen = "\
Allow the rig MCP server to run tool agent_send?

› 1. Allow
  2. Allow for this session
  3. Always allow
  4. Cancel
";
    let found = recognise(CODEX_DIALOGS, screen).expect("mcp");
    let cwd = home_cwd();
    let seat = SeatContext {
        cwd: &cwd,
        home: &cwd,
    };
    assert_eq!(
        plan(&found, approve_policy(true), &seat),
        Plan::Answer("Allow for this session")
    );
    let spike = screen.replace("agent_send", "seat_send");
    let spike = recognise(CODEX_DIALOGS, &spike).expect("spike");
    assert!(matches!(
        plan(&spike, approve_policy(true), &seat),
        Plan::Escalate(_)
    ));
    assert!(matches!(
        plan(&found, approve_policy(false), &seat),
        Plan::Escalate(_)
    ));
}

fn approve_policy(auto_approve_rig_tools: bool) -> AnswerPolicy {
    AnswerPolicy {
        auto_trust: false,
        auto_approve_rig_tools,
    }
}

fn home_cwd() -> std::path::PathBuf {
    std::env::temp_dir()
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
