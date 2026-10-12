//! Submit requires a ready composer and a byte-exact rollout user item.

#[path = "codex_support.rs"]
mod support;

use std::fs;
use std::io::Write;
use std::sync::Arc;

use crewmux_adapters::{AgentAdapter, ResumeToken, SubmitMode};
use crewmux_mux::{Capture, FakeMux};
use crewmux_types::ReadinessCode;

use support::{
    LOGGED_IN, READY, adapter, append_user, binding, calls_include, home_with, opts, write_rollout,
};

#[tokio::test]
async fn submit_matches_the_rollout_byte_for_byte() {
    let body = "hello\nworld";
    let (seat, binding, home, _mux) = ready_seat().await;
    let path = write_rollout(&home.codex_dir, "thread-a", &home.cwd, None);
    let finished = body.to_owned();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        append_user(&path, &finished);
    });
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
    let (seat, binding, home, _mux) = ready_seat().await;
    let path = write_rollout(&home.codex_dir, "thread-a", &home.cwd, None);
    let finished = body.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        append_user(&path, &finished);
    });
    let receipt = seat
        .submit(&binding, &body, SubmitMode::Enter)
        .await
        .expect("submit");
    assert_eq!(receipt.bytes, 64 * 1024);
}

#[tokio::test]
async fn earlier_identical_user_text_does_not_confirm_submit() {
    let (seat, binding, home, _mux) = ready_seat().await;
    write_rollout(&home.codex_dir, "thread-a", &home.cwd, Some("yes"));
    let err = seat
        .submit(&binding, "yes", SubmitMode::Enter)
        .await
        .expect_err("old yes");
    assert!(err.to_string().contains("rollout"), "{err}");
    assert!(!err.to_string().contains("malformed"), "{err}");
}

#[tokio::test]
async fn missing_rollout_item_is_not_verified() {
    let (seat, binding, _home, _mux) = ready_seat().await;
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

#[tokio::test]
async fn submit_refuses_enter_when_the_composer_stays_empty() {
    let (seat, binding, _home, mux) = ready_seat().await;
    mux.set_capture_after_paste(Capture::new(READY.to_owned(), (0, 1)))
        .await;
    let before = mux.recorded().await.len();
    let err = seat
        .submit(&binding, "hello", SubmitMode::Enter)
        .await
        .expect_err("empty");
    assert!(err.to_string().contains("composer"));
    let extra = &mux.recorded().await[before..];
    assert!(!calls_include(extra, "send-keys"));
}

#[tokio::test]
async fn submit_refuses_enter_when_the_draft_is_not_the_body() {
    let (seat, binding, _home, mux) = ready_seat().await;
    mux.set_capture_after_paste(Capture::new(
        "╭──╮\n│ › nope │\n╰──╯\n  ? for shortcuts\n".to_owned(),
        (0, 1),
    ))
    .await;
    let before = mux.recorded().await.len();
    let err = seat
        .submit(&binding, "hello", SubmitMode::Enter)
        .await
        .expect_err("mismatch");
    assert!(err.to_string().contains("paste"));
    let extra = &mux.recorded().await[before..];
    assert!(!calls_include(extra, "send-keys"));
}

#[tokio::test]
async fn partial_last_line_is_skipped_until_it_completes() {
    let body = "hello\nworld";
    let (seat, binding, home, mux) = ready_seat().await;
    let path = write_rollout(&home.codex_dir, "thread-a", &home.cwd, None);
    let finished = body.to_owned();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        let line = serde_json::json!({
            "type": "response_item",
            "payload": {"role": "user", "text": finished}
        })
        .to_string();
        let mid = line.len() / 2;
        let Some((head, tail)) = line.split_at_checked(mid) else {
            return;
        };
        // Append only. A rewrite truncates the file, and the submit poller
        // can observe an empty file and treat it as malformed.
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("rollout");
        file.write_all(head.as_bytes()).expect("partial");
        drop(file);
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("rollout");
        file.write_all(tail.as_bytes()).expect("tail");
        file.write_all(b"\n").expect("newline");
    });
    let receipt = seat
        .submit(&binding, body, SubmitMode::Enter)
        .await
        .expect("submit");
    assert_eq!(receipt.bytes, body.len());
    assert!(calls_include(&mux.recorded().await, "send-keys"));
}

#[tokio::test]
async fn partial_last_line_is_not_malformed_after_enter() {
    let (seat, binding, home, _mux) = ready_seat().await;
    let path = write_rollout(&home.codex_dir, "thread-a", &home.cwd, None);
    let mut partial = fs::read_to_string(&path).expect("rollout");
    partial.push_str("{\"payload\":{\"text\":\"hel\"");
    fs::write(&path, partial).expect("partial");
    let err = seat
        .submit(&binding, "hello", SubmitMode::Enter)
        .await
        .expect_err("waiting");
    assert!(!err.to_string().contains("malformed"), "{}", err);
    assert!(err.to_string().contains("rollout"));
}

#[tokio::test]
async fn resume_rejects_a_bad_token_without_echoing_it() {
    let home = home_with(LOGGED_IN);
    let mux = Arc::new(FakeMux::new());
    let seat = adapter(&mux, &home.bin);
    let binding = binding(&home.cwd, "seat");
    let mut launch = opts(&home, "seat");
    let secret = "/tmp/secret-token";
    launch.resume = Some(ResumeToken {
        kind: "codex".to_owned(),
        value: secret.to_owned(),
    });
    let err = seat.launch(&binding, launch).await.expect_err("token");
    assert!(!err.to_string().contains(secret), "{err}");
    assert!(err.to_string().contains("disallowed"), "{err}");
    assert!(!calls_include(&mux.recorded().await, "new-session"));
}

#[expect(
    clippy::expect_used,
    reason = "a failed launch in the submit fixture fails the test"
)]
async fn ready_seat() -> (
    crewmux_adapters::CodexAdapter,
    crewmux_adapters::NodeBinding,
    support::Home,
    Arc<FakeMux>,
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
    (seat, binding, home, mux)
}
