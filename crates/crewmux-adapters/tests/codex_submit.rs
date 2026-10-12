//! Submit requires a ready composer and a byte-exact rollout user item.

#[path = "common/mod.rs"]
mod support;

use std::sync::Arc;

use crewmux_adapters::{AgentAdapter, SubmitMode};
use crewmux_mux::{Capture, FakeMux};
use crewmux_types::ReadinessCode;

use support::{LOGGED_IN, READY, adapter, binding, calls_include, home_with, opts, write_rollout};

#[tokio::test]
async fn submit_matches_the_rollout_byte_for_byte() {
    let body = "hello\nworld";
    let (seat, binding, home) = ready_seat().await;
    write_rollout(&home.codex_dir, "thread-a", &home.cwd, Some(body));
    let receipt = seat
        .submit(&binding, body, SubmitMode::Enter)
        .await
        .expect("submit");
    assert_eq!(receipt.bytes, body.len());
    assert_eq!(
        seat.thread_id(binding.seat).await.expect("id").as_str(),
        "thread-a"
    );
}

#[tokio::test]
async fn submit_accepts_a_64kb_body() {
    let body = "x".repeat(64 * 1024);
    let (seat, binding, home) = ready_seat().await;
    write_rollout(&home.codex_dir, "thread-a", &home.cwd, Some(&body));
    let receipt = seat
        .submit(&binding, &body, SubmitMode::Enter)
        .await
        .expect("submit");
    assert_eq!(receipt.bytes, 64 * 1024);
}

#[tokio::test]
async fn missing_rollout_item_is_not_verified() {
    let (seat, binding, _home) = ready_seat().await;
    let err = seat
        .submit(&binding, "nothing landed", SubmitMode::Enter)
        .await
        .expect_err("timeout");
    assert!(err.to_string().contains("rollout"));
    let verdict = seat.check_ready(&binding).await.expect("ready");
    assert_eq!(
        verdict,
        crewmux_adapters::Readiness::Gate(ReadinessCode::NotVerified)
    );
}

#[tokio::test]
async fn submit_refuses_a_login_menu() {
    let home = home_with(LOGGED_IN);
    let mux = Arc::new(FakeMux::new());
    let seat = adapter(&mux, &home.bin);
    let binding = binding(&home.cwd, "seat");
    seat.launch(&binding, opts(&home, "seat"))
        .await
        .expect("launch");
    let before = mux.recorded().await.len();
    mux.set_capture(Capture::new(
        "Sign in with ChatGPT\nProvide your own API key\n".to_owned(),
        (0, 0),
    ))
    .await;
    let err = seat
        .submit(&binding, "nope", SubmitMode::Enter)
        .await
        .expect_err("login");
    assert!(err.to_string().contains("ready"));
    let extra = &mux.recorded().await[before..];
    assert!(!calls_include(extra, "send-keys"));
    assert!(!calls_include(extra, "paste-buffer"));
}

#[expect(
    clippy::expect_used,
    reason = "a failed launch in the submit fixture fails the test"
)]
async fn ready_seat() -> (
    crewmux_adapters::CodexAdapter,
    crewmux_adapters::NodeBinding,
    support::Home,
) {
    let home = home_with(LOGGED_IN);
    let mux = Arc::new(FakeMux::new());
    let seat = adapter(&mux, &home.bin);
    let binding = binding(&home.cwd, "seat");
    seat.launch(&binding, opts(&home, "seat"))
        .await
        .expect("launch");
    mux.set_capture(Capture::new(READY.to_owned(), (0, 1)))
        .await;
    (seat, binding, home)
}
