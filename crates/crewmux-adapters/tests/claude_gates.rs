//! PE-001: first-run and other gates are attention, and nothing is pasted.
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "temp-dir setup failures should fail the test"
)]

use std::sync::Arc;

use crewmux_adapters::{
    ClaudeAdapter, Error, Readiness, SeatIo, SubmitMode,
    claude::{ClaudeScreen, classify_text, readiness},
};
use crewmux_mux::FakeMux;
use crewmux_types::{ReadinessCode, SeatId};

#[path = "common/mod.rs"]
mod support;

#[tokio::test]
async fn first_run_screens_are_not_ready() {
    let cases = [
        ("cl-0-theme.txt", ReadinessCode::OnboardingTheme),
        ("cl-1-login-method.txt", ReadinessCode::LoginRequired),
        ("cl-2-login-method-opt2.txt", ReadinessCode::LoginRequired),
    ];
    for (name, code) in cases {
        let text = support::read(&support::fixtures().join(name));
        let screen = classify_text(&text);
        assert!(screen.requires_attention(), "{name}");
        assert!(!screen.is_ready(), "{name}");
        assert_eq!(screen.code(), Some(code.clone()), "{name}");
        assert_eq!(readiness(&screen), Readiness::Gate(code));
        assert_no_paste(&text).await;
    }
}

#[test]
fn theme_picker_is_not_ready() {
    let screen = classify_text(&support::read(&support::fixtures().join("cl-0-theme.txt")));
    assert!(matches!(screen, ClaudeScreen::Login(_)));
    assert!(!screen.is_ready());
}

#[test]
fn mcp_server_found_is_attention() {
    let screen = classify_named("claude-055-mcpgate-after-trust.txt");
    assert_eq!(screen.code(), Some(ReadinessCode::McpGate));
    assert!(screen.requires_attention());
    assert_eq!(
        crewmux_adapters::claude::strict_mcp_drift(&screen),
        Some("mcp dialog appeared despite --strict-mcp-config")
    );
}

#[test]
fn fullscreen_renderer_popup_is_attention() {
    let screen = classify_named("claude-007-fullscreen.txt");
    assert_eq!(screen.code(), Some(ReadinessCode::FullscreenRenderer));
    assert!(screen.requires_attention());
    assert!(!screen.is_ready());
}

#[test]
fn ready_requires_footer_and_no_menu() {
    let ready = classify_named("claude-020-ready.txt");
    assert!(ready.is_ready());
    assert!(!ready.requires_attention());
    let splash = classify_named("claude-019-splash.txt");
    assert!(!splash.is_ready());
    assert!(matches!(splash, ClaudeScreen::Inconclusive));
}

#[test]
fn midturn_is_busy_not_ready() {
    let screen = classify_named("claude-040-midturn.txt");
    assert!(matches!(screen, ClaudeScreen::Busy));
    assert!(!screen.is_ready());
    assert_eq!(readiness(&screen), Readiness::Booting);
}

#[test]
fn splash_without_footer_is_not_ready() {
    let screen = classify_named("claude-019-splash.txt");
    assert!(matches!(screen, ClaudeScreen::Inconclusive));
    assert!(!screen.is_ready());
    assert!(!screen.requires_attention());
    assert_eq!(readiness(&screen), Readiness::Inconclusive);
}

fn classify_named(name: &str) -> ClaudeScreen {
    classify_text(&support::read(
        &support::fixtures().join("2.1.286").join(name),
    ))
}

async fn assert_no_paste(text: &str) {
    let mux = Arc::new(FakeMux::new());
    mux.set_capture(support::capture(text)).await;
    let root = std::env::temp_dir().join(format!("rig-gates-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&root));
    let seats = root.join("seats");
    let adapter = ClaudeAdapter::new(
        support::shared(&mux),
        support::null_check(),
        seats,
        root.join("home"),
    );
    let seat = SeatId::new();
    let binding = crewmux_adapters::binding_for(seat, root.clone(), "gate");
    std::fs::create_dir_all(adapter.seat_dir(&binding.seat)).expect("seat dir");
    std::fs::write(
        adapter.seat_dir(&binding.seat).join("claude-session-id"),
        "sid",
    )
    .expect("id");
    let io = adapter.io_for(&binding).await.expect("io");
    let err = io
        .submit("payload 1", SubmitMode::Enter)
        .await
        .expect_err("blocked");
    assert!(matches!(err, Error::NotReady));
    let calls = mux.recorded().await;
    assert!(!support::sent_paste(&calls), "{calls:?}");
    assert!(!support::sent_enter(&calls), "{calls:?}");
    drop(std::fs::remove_dir_all(&root));
}
