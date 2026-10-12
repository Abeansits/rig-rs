//! Labelled Codex screen corpus.
#![expect(
    clippy::expect_used,
    reason = "a missing screen fixture fails the test"
)]
#![expect(clippy::panic, reason = "a mislabelled capture fails the test")]
#![expect(
    clippy::wildcard_enum_match_arm,
    reason = "a capture must be the labelled screen and nothing else"
)]

#[path = "common/mod.rs"]
mod support;

use std::fs;

use crewmux_adapters::codex::{CodexScreen, classify};
use crewmux_adapters::dialog::KnownDialog;
use proptest::prelude::*;
use serde_json::Value;

use support::fixtures;

#[test]
fn every_labelled_capture_matches() {
    let dir = fixtures().join("screens/codex/0.162.0");
    let labels: Vec<Value> =
        serde_json::from_str(&fs::read_to_string(dir.join("labels.json")).expect("labels"))
            .expect("json");
    let mut labelled = Vec::new();
    for label in &labels {
        let file = label["file"].as_str().expect("file");
        labelled.push(file.to_owned());
        let text = fs::read_to_string(dir.join(file)).expect(file);
        assert_screen(label["screen"].as_str().expect("screen"), &text, label);
    }
    for entry in fs::read_dir(&dir).expect("dir") {
        let name = entry.expect("entry").file_name();
        let name = name.to_string_lossy().to_string();
        if std::path::Path::new(&name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"))
        {
            assert!(
                labelled.iter().any(|file| file == &name),
                "unlabelled {name}"
            );
        }
    }
}

#[test]
fn midturn_is_busy_not_ready() {
    let text = fs::read_to_string(fixtures().join("screens/codex/0.162.0/codex-040-midturn.txt"))
        .expect("midturn");
    assert!(matches!(classify(&text), CodexScreen::Busy));
    assert!(text.contains("? for shortcuts"));
}

#[test]
fn shell_prompt_is_not_ready() {
    assert!(matches!(
        classify("codex exited\n$"),
        CodexScreen::ReturnedToShell
    ));
}

#[test]
fn draft_composer_is_not_ready() {
    let text = "╭──╮\n│ › hello │\n╰──╯\n  ? for shortcuts\n";
    match classify(text) {
        CodexScreen::Draft { visible } => assert_eq!(visible, "hello"),
        other => panic!("draft was {other:?}"),
    }
}

#[test]
fn unparsed_composer_is_not_ready() {
    let text = "╭──╮\n│ working │\n╰──╯\n  ? for shortcuts\n";
    assert!(matches!(classify(text), CodexScreen::Inconclusive));
}

#[test]
fn ready_composer_ignores_scrollback_update_banner() {
    let text = "\
Update available! 0.162.0 → 0.162.1
› 1. Update now (runs brew upgrade --cask codex)
  2. Skip
  3. Skip until next version
╭──╮
│ ›  │
╰──╯
  ? for shortcuts
";
    assert!(matches!(classify(text), CodexScreen::Ready(_)));
}

#[test]
fn update_title_inside_prose_is_not_a_dialog() {
    let text = "\
The notes mention Update available in the changelog.
› 1. Update now (runs brew upgrade --cask codex)
  2. Skip
  3. Skip until next version
";
    assert!(matches!(classify(text), CodexScreen::UnknownMenu { .. }));
}

#[test]
fn dialog_title_after_the_composer_stays_a_dialog() {
    let text = "\
╭──╮
│ ›  │
╰──╯
  ? for shortcuts
Update available! 0.162.0 → 0.162.1
› 1. Update now (runs brew upgrade --cask codex)
  2. Skip
  3. Skip until next version
";
    assert!(matches!(
        classify(text),
        CodexScreen::Dialog(KnownDialog::CodexUpdate { .. })
    ));
}

#[test]
fn numbered_menu_is_not_ready() {
    let text = "Select a model\n› 1. gpt-5.1\n  2. gpt-5.1-codex\n";
    assert!(matches!(classify(text), CodexScreen::UnknownMenu { .. }));
}

#[test]
fn trust_prompt_is_not_ready() {
    let text = "Accessing workspace\n/tmp/work\n› 1. Yes, I trust this folder\n  2. No, exit\n";
    assert!(matches!(
        classify(text),
        CodexScreen::Dialog(KnownDialog::ClaudeTrust { .. })
    ));
}

proptest::proptest! {
    #[test]
    fn interrupt_marker_is_never_ready(tail in "[a-z ]{0,24}") {
        let text = format!("esc to interrupt\n? for shortcuts\n{tail}");
        let screen = classify(&text);
        prop_assert!(matches!(screen, CodexScreen::Busy));
    }

    #[test]
    fn sign_in_menu_is_never_ready(pad in "[a-z]{0,12}") {
        let text = format!("{pad}\nSign in with ChatGPT\nProvide your own API key\n");
        let screen = classify(&text);
        prop_assert!(matches!(screen, CodexScreen::Login(_)));
    }
}

fn assert_screen(kind: &str, text: &str, label: &Value) {
    match kind {
        "ready" => assert!(matches!(classify(text), CodexScreen::Ready(_))),
        "busy" => assert!(matches!(classify(text), CodexScreen::Busy)),
        "stale" => assert!(matches!(classify(text), CodexScreen::StaleServer)),
        "background" => assert!(matches!(
            classify(text),
            CodexScreen::Dialog(KnownDialog::CodexBackgroundServer)
        )),
        "update" => match classify(text) {
            CodexScreen::Dialog(KnownDialog::CodexUpdate { from, to }) => {
                assert_eq!(from, label["from"].as_str().expect("from"));
                assert_eq!(to, label["to"].as_str().expect("to"));
            }
            other => panic!("update was {other:?}"),
        },
        "mcp" => match classify(text) {
            CodexScreen::Dialog(KnownDialog::CodexMcpToolApproval { server, tool }) => {
                assert_eq!(server, label["server"].as_str().expect("server"));
                assert_eq!(tool, label["tool"].as_str().expect("tool"));
            }
            other => panic!("mcp was {other:?}"),
        },
        other => panic!("unknown label {other}"),
    }
}
