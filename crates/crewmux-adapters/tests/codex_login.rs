//! Login pre-flight and the logged-out screen corpus.

#[path = "common/mod.rs"]
mod support;

use std::fs;
use std::sync::Arc;

use crewmux_adapters::codex::{CodexScreen, Preflight, classify, classify_preflight};
use crewmux_adapters::{AgentAdapter, LaunchOutcome, Readiness};
use crewmux_mux::{Capture, FakeMux};
use crewmux_types::{LoginKind, ReadinessCode};
use serde_json::Value;

use support::{
    MISSING_HOME, NOT_LOGGED_IN, READY, adapter, binding, calls_include, fixtures, home_with, opts,
};

#[test]
fn all_live_logged_out_screens_are_login_required() {
    let dir = fixtures().join("screens/codex/0.162.1-logged-out");
    let index: Vec<Value> =
        serde_json::from_str(&fs::read_to_string(dir.join("index.json")).expect("index"))
            .expect("rows");
    assert!(index.len() >= 54, "corpus {}", index.len());
    for row in index {
        let file = row["file"].as_str().expect("file").to_owned();
        let expect = row["expect"].as_str().expect("expect");
        let live = row["live"].as_bool().expect("live");
        let text = fs::read_to_string(dir.join(&file)).expect(&file);
        let screen = classify(&text);
        assert_not_waiting(&file, &screen);
        if live {
            assert_eq!(login_token(&screen), Some(expect), "{file}");
        } else {
            assert!(is_dead(&screen), "{file} => {screen:?}");
        }
    }
}

#[tokio::test]
async fn preflight_exit1_short_circuits() {
    let home = home_with(NOT_LOGGED_IN);
    let mux = Arc::new(FakeMux::new());
    let adapter = adapter(&mux, &home.bin);
    let binding = binding(&home.cwd, "seat");
    let outcome = adapter
        .launch(&binding, opts(&home, "seat"))
        .await
        .expect("launch");
    assert!(matches!(
        outcome,
        LaunchOutcome::LoginRequired {
            kind: LoginKind::PreflightNotLoggedIn,
            ..
        }
    ));
    let calls = mux.recorded().await;
    assert!(!calls_include(&calls, "new-session"));
    assert!(!calls_include(&calls, "send-keys"));
}

#[tokio::test]
async fn codex_home_missing_is_hard_failure() {
    let home = home_with(MISSING_HOME);
    let mux = Arc::new(FakeMux::new());
    let adapter = adapter(&mux, &home.bin);
    let binding = binding(&home.cwd, "seat");
    let outcome = adapter
        .launch(&binding, opts(&home, "seat"))
        .await
        .expect("launch");
    match outcome {
        LaunchOutcome::Failed { error, .. } => assert!(error.contains("codex_home_missing")),
        other @ (LaunchOutcome::Ok { .. } | LaunchOutcome::LoginRequired { .. }) => {
            panic!("expected hard failure, got {other:?}");
        }
    }
    assert!(!calls_include(&mux.recorded().await, "new-session"));
    let verdict = adapter.check_ready(&binding).await.expect("ready");
    assert_eq!(
        verdict,
        Readiness::HardFailure("codex_home_missing".to_owned())
    );
}

#[tokio::test]
async fn no_keys_sent_while_login_required() {
    let home = home_with(NOT_LOGGED_IN);
    let mux = Arc::new(FakeMux::new());
    let adapter = adapter(&mux, &home.bin);
    let binding = binding(&home.cwd, "seat");
    adapter
        .launch(&binding, opts(&home, "seat"))
        .await
        .expect("launch");
    mux.set_capture(Capture::new(menu(), (0, 0))).await;
    let verdict = adapter.check_ready(&binding).await.expect("ready");
    assert!(matches!(
        verdict,
        Readiness::Login {
            kind: LoginKind::PreflightNotLoggedIn,
            ..
        }
    ));
    let calls = mux.recorded().await;
    assert!(!calls_include(&calls, "send-keys"));
    assert!(!calls_include(&calls, "paste-buffer"));
    assert!(adapter.interrupt(&binding).is_err());
    assert!(!calls_include(&mux.recorded().await, "send-keys"));
}

#[tokio::test]
async fn preflight_not_logged_in_never_ready_even_with_composer() {
    let home = home_with(NOT_LOGGED_IN);
    let mux = Arc::new(FakeMux::new());
    let adapter = adapter(&mux, &home.bin);
    let binding = binding(&home.cwd, "seat");
    adapter
        .launch(&binding, opts(&home, "seat"))
        .await
        .expect("launch");
    mux.set_capture(Capture::new(READY.to_owned(), (0, 1)))
        .await;
    let verdict = adapter.check_ready(&binding).await.expect("ready");
    assert!(matches!(verdict, Readiness::Login { .. }));
    assert!(!matches!(verdict, Readiness::Ready));
}

#[tokio::test]
async fn unrecognised_preflight_is_recorded_and_launch_continues() {
    let home = home_with(support::UNRECOGNISED);
    let mux = Arc::new(FakeMux::new());
    let adapter = adapter(&mux, &home.bin);
    let binding = binding(&home.cwd, "seat");
    let outcome = adapter
        .launch(&binding, opts(&home, "seat"))
        .await
        .expect("launch");
    assert!(matches!(outcome, LaunchOutcome::Ok { .. }));
    let note = adapter.preflight_note(binding.seat).await.expect("note");
    assert!(note.contains("preflight_unrecognised"));
    assert!(calls_include(&mux.recorded().await, "new-session"));
}

#[tokio::test]
async fn stale_server_text_is_attention() {
    let text = fs::read_to_string(fixtures().join("screens/codex/0.162.0/codex-stale.txt"))
        .expect("stale");
    assert!(matches!(classify(&text), CodexScreen::StaleServer));
    let home = home_with(support::LOGGED_IN);
    let mux = Arc::new(FakeMux::new());
    let adapter = adapter(&mux, &home.bin);
    let binding = binding(&home.cwd, "seat");
    adapter
        .launch(&binding, opts(&home, "seat"))
        .await
        .expect("launch");
    mux.set_capture(Capture::new(text, (0, 0))).await;
    let verdict = adapter.check_ready(&binding).await.expect("ready");
    assert_eq!(verdict, Readiness::Gate(ReadinessCode::StaleCodexServer));
    assert!(!calls_include(&mux.recorded().await, "send-keys"));
}

#[test]
fn captured_login_status_text_is_exit_1() {
    let dir = fixtures().join("screens/codex/0.162.1-logged-out");
    let text = fs::read_dir(&dir)
        .expect("dir")
        .filter_map(Result::ok)
        .map(|entry| fs::read_to_string(entry.path()).unwrap_or_default())
        .find(|body| body.contains("Not logged in"))
        .expect("login status capture");
    assert_eq!(classify_preflight(1, &text), Preflight::NotLoggedIn);
    assert_eq!(
        classify_preflight(1, "Error finding codex home config.toml: does not exist\n"),
        Preflight::CodexHomeMissing
    );
}

#[tokio::test]
async fn throwaway_home_is_not_logged_in_when_codex_is_installed() {
    let version = tokio::process::Command::new("codex")
        .arg("--version")
        .output()
        .await;
    let Ok(version) = version else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let dir = tempfile::tempdir().expect("temp");
    let home = dir.path().join("home");
    let codex_home = home.join(".codex");
    fs::create_dir_all(&codex_home).expect("dir");
    let preflight = crewmux_adapters::codex::codex_preflight(&home, &codex_home).await;
    assert!(
        matches!(
            preflight,
            Preflight::NotLoggedIn | Preflight::CodexHomeMissing
        ),
        "throwaway CODEX_HOME must not look logged in: {preflight:?}"
    );
}

fn assert_not_waiting(file: &str, screen: &CodexScreen) {
    assert!(!matches!(screen, CodexScreen::Ready(_)), "{file} ready");
    assert!(
        !matches!(screen, CodexScreen::Inconclusive),
        "{file} inconclusive"
    );
    assert!(
        !matches!(screen, CodexScreen::UnknownMenu { .. }),
        "{file} menu"
    );
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "only login screens carry a corpus token"
)]
fn login_token(screen: &CodexScreen) -> Option<&'static str> {
    match screen {
        CodexScreen::Login(LoginKind::Menu) => Some("menu"),
        CodexScreen::Login(LoginKind::ChatGptBrowser) => Some("chatgpt_browser"),
        CodexScreen::Login(LoginKind::DeviceCode) => Some("device_code"),
        CodexScreen::Login(LoginKind::ApiKeyEntry) => Some("api_key_entry"),
        _ => None,
    }
}

fn is_dead(screen: &CodexScreen) -> bool {
    matches!(
        screen,
        CodexScreen::ReturnedToShell | CodexScreen::HardFailure { .. }
    )
}

fn menu() -> String {
    "Sign in with ChatGPT\nProvide your own API key\n".to_owned()
}
