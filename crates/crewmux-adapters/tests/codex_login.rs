//! Login pre-flight and the logged-out screen corpus.

#[path = "codex_support.rs"]
mod support;

use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crewmux_adapters::codex::{CodexScreen, LaunchBudget, Preflight, classify, classify_preflight};
use crewmux_adapters::{AgentAdapter, LaunchOutcome, Readiness};
use crewmux_mux::{Capture, FakeMux};
use crewmux_types::{LoginKind, ReadinessCode};
use serde_json::Value;

use support::{
    MISSING_HOME, NOT_LOGGED_IN, READY, adapter, binding, calls_include, fixtures, home_with, opts,
};

#[test]
fn all_live_logged_out_screens_are_login_required() {
    let dir = fixtures().join("0.162.1-logged-out");
    let index: Vec<Value> =
        serde_json::from_str(&fs::read_to_string(dir.join("index.json")).expect("index"))
            .expect("rows");
    assert_eq!(index.len(), 54, "corpus {}", index.len());
    let labels = fs::read_to_string(dir.join("labels.jsonl")).expect("sha list");
    let mut sha_rows = std::collections::BTreeMap::new();
    for line in labels.lines() {
        let row: Value = serde_json::from_str(line).expect("sha row");
        sha_rows.insert(
            row["file"].as_str().expect("file").to_owned(),
            row["sha256"].as_str().expect("sha").to_owned(),
        );
    }
    let mut dead = 0usize;
    for row in index {
        let file = row["file"].as_str().expect("file").to_owned();
        let pane_dead = row["pane_dead"].as_str().expect("pane_dead") == "1";
        let text = fs::read_to_string(dir.join(&file)).expect(&file);
        let sha = crewmux_transcript::fingerprint_raw(&text);
        assert_eq!(
            sha_rows.get(&file).map(String::as_str),
            Some(sha.as_str()),
            "{file} sha"
        );
        let screen = classify(&text);
        assert_not_waiting(&file, &screen);
        if pane_dead {
            dead += 1;
            assert!(is_dead(&screen), "{file} => {screen:?}");
        } else {
            let kind = login_expect(&text).unwrap_or_else(|| panic!("{file} has no login phrase"));
            assert_eq!(login_token(&screen), Some(kind), "{file}");
        }
    }
    assert_eq!(dead, 8, "dead panes");
    assert!(
        sha_rows.contains_key("login-status-stale.txt"),
        "stale login status is labelled"
    );
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
async fn unrecognised_preflight_refuses_the_session() {
    let home = home_with(support::UNRECOGNISED);
    let mux = Arc::new(FakeMux::new());
    let adapter = adapter(&mux, &home.bin);
    let binding = binding(&home.cwd, "seat");
    let outcome = adapter
        .launch(&binding, opts(&home, "seat"))
        .await
        .expect("launch");
    match outcome {
        LaunchOutcome::Failed { error, .. } => {
            assert!(error.contains("codex login status failed"), "{error}");
            assert!(error.contains("status unavailable"), "{error}");
        }
        other @ (LaunchOutcome::Ok { .. } | LaunchOutcome::LoginRequired { .. }) => {
            panic!("expected a preflight failure, got {other:?}");
        }
    }
    assert!(!calls_include(&mux.recorded().await, "new-session"));
}

#[tokio::test]
async fn missing_codex_binary_refuses_the_session() {
    let home = home_with(support::LOGGED_IN);
    fs::remove_file(&home.bin).expect("remove");
    let mux = Arc::new(FakeMux::new());
    let adapter = adapter(&mux, &home.bin);
    let binding = binding(&home.cwd, "seat");
    let outcome = adapter
        .launch(&binding, opts(&home, "seat"))
        .await
        .expect("launch");
    match outcome {
        LaunchOutcome::Failed { error, .. } => {
            assert!(error.contains("codex login status failed"), "{error}");
        }
        other @ (LaunchOutcome::Ok { .. } | LaunchOutcome::LoginRequired { .. }) => {
            panic!("expected a missing binary, got {other:?}");
        }
    }
    assert!(!calls_include(&mux.recorded().await, "new-session"));
}

#[tokio::test]
async fn hanging_login_status_times_out_without_a_session() {
    let home = home_with(support::HANGING);
    let mux = Arc::new(FakeMux::new());
    let adapter = adapter(&mux, &home.bin).with_budget(LaunchBudget {
        preflight: Duration::from_millis(200),
        ..support::fast_budget()
    });
    let binding = binding(&home.cwd, "seat");
    let started = Instant::now();
    let outcome = adapter
        .launch(&binding, opts(&home, "seat"))
        .await
        .expect("launch");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "login status hung for {:?}",
        started.elapsed()
    );
    match outcome {
        LaunchOutcome::Failed { error, .. } => assert!(error.contains("timed out"), "{error}"),
        other @ (LaunchOutcome::Ok { .. } | LaunchOutcome::LoginRequired { .. }) => {
            panic!("expected a timeout, got {other:?}");
        }
    }
    assert!(!calls_include(&mux.recorded().await, "new-session"));
}

#[tokio::test]
async fn stale_server_text_is_attention() {
    let text = "\
╭──╮
│ ›  │
╰──╯
control socket is already in use
"
    .to_owned();
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
    let dir = fixtures().join("0.162.1-logged-out");
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

fn login_expect(text: &str) -> Option<&'static str> {
    if text.contains("Enter this one-time code") {
        Some("device_code")
    } else if text.contains("Finish signing in via your browser") {
        Some("chatgpt_browser")
    } else if text.contains("Paste or type your API key") {
        Some("api_key_entry")
    } else if text.contains("Sign in with ChatGPT") && text.contains("Provide your own API key") {
        Some("menu")
    } else {
        None
    }
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
