//! Cursor-verified dialog answers. Enter is never the first key on a default "No, exit".

#[path = "common/mod.rs"]
mod support;

use crewmux_adapters::codex::classify;
use crewmux_adapters::dialog::{
    AnswerPolicy, DialogChoice, DriveResult, KnownDialog, menu_options, planned_choice,
};
use crewmux_mux::Key;

use support::{Motion, drive_menu};

#[tokio::test]
async fn never_enter_on_default_no_exit() {
    let report = drive_menu(
        "Accessing workspace",
        &["No, exit", "Yes, I trust this folder"],
        DialogChoice::TrustFolder,
        Motion::Follow,
    )
    .await;
    assert_eq!(report.result, DriveResult::Answered);
    assert_eq!(report.keys.first(), Some(&Key::Down));
    assert_ne!(report.keys.first(), Some(&Key::Enter));
    let before = report.before_enter.expect("enter");
    let selected = menu_options(&before)
        .into_iter()
        .find(|opt| opt.selected)
        .expect("cursor");
    assert_eq!(selected.label, "Yes, I trust this folder");
}

#[test]
fn trust_not_answered_without_opt_in() {
    assert!(planned_choice(&KnownDialog::ClaudeTrust, AnswerPolicy::default()).is_none());
    let opted = AnswerPolicy {
        auto_trust: true,
        auto_approve_rig_tools: false,
    };
    assert_eq!(
        planned_choice(&KnownDialog::ClaudeTrust, opted),
        Some(DialogChoice::TrustFolder)
    );
}

#[tokio::test]
async fn codex_update_answers_skip_never_update() {
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
    assert_eq!(report.result, DriveResult::Answered);
    let before = report.before_enter.expect("enter");
    let selected = menu_options(&before)
        .into_iter()
        .find(|opt| opt.selected)
        .expect("cursor");
    assert_eq!(selected.label, "Skip");
    assert!(!selected.label.contains("Update now"));
}

#[test]
fn bypass_always_escalates() {
    assert!(planned_choice(&KnownDialog::ClaudeBypass, AnswerPolicy::default()).is_none());
}

#[test]
fn unknown_menu_escalates() {
    let text = "Pick one\n  1. Alpha\n› 2. Beta\n";
    assert!(matches!(
        classify(text),
        crewmux_adapters::codex::CodexScreen::UnknownMenu { .. }
    ));
    assert!(planned_choice(&KnownDialog::CodexBackgroundServer, AnswerPolicy::default()).is_none());
}

#[tokio::test]
async fn cursor_verification_failure_escalates() {
    let report = drive_menu(
        "Update available! 0.162.0 → 0.162.1",
        &[
            "Update now (runs brew upgrade --cask codex)",
            "Skip",
            "Skip until next version",
        ],
        DialogChoice::Skip,
        Motion::Wrong,
    )
    .await;
    assert!(matches!(report.result, DriveResult::Escalated { .. }));
    assert!(!report.keys.contains(&Key::Enter));
    assert!(!report.keys.contains(&Key::Escape));
    assert!(!report.keys.contains(&Key::CtrlC));
}

#[tokio::test]
async fn no_quit_keys_on_login_menu() {
    let report = drive_menu(
        "Sign in with ChatGPT",
        &["Sign in with ChatGPT", "Provide your own API key"],
        DialogChoice::Skip,
        Motion::Stuck,
    )
    .await;
    assert!(matches!(report.result, DriveResult::Escalated { .. }));
    assert!(
        report
            .keys
            .iter()
            .all(|key| matches!(key, Key::Up | Key::Down))
    );
}
