//! Claude Code 2.1.286 screen corpus. Fixtures are reconstructed, not spike bytes.
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "a mismatched fixture should fail the test"
)]

use std::collections::BTreeMap;
use std::path::Path;

use crewmux_adapters::claude::{ClaudeScreen, LoginKind, classify_text};
use crewmux_types::KnownDialog;
use proptest::prelude::*;
use serde_json::Value;

#[path = "common/mod.rs"]
mod support;

#[test]
fn every_corpus_file_is_labelled() {
    let dir = support::fixtures().join("2.1.286");
    let labels = load_labels(&dir);
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("corpus") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("txt") {
            files.push(path);
        }
    }
    assert_ne!(files.len(), 0);
    for path in files {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("name");
        let label = labels
            .get(name)
            .unwrap_or_else(|| panic!("unlabelled {name}"));
        let screen = classify_text(&support::read(&path));
        assert_label(&screen, label, name);
    }
}

#[test]
fn ac1_firstrun_is_theme_picker() {
    expect_login("claude-000-firstrun.txt", LoginKind::ThemePicker);
    expect_login("claude-001-firstrun.txt", LoginKind::ThemePicker);
    expect_login("claude-002-firstrun.txt", LoginKind::ThemePicker);
    expect_login("claude-003-firstrun.txt", LoginKind::ThemePicker);
}

#[test]
fn ac2_trust_dialog_defaults_to_no_exit() {
    for name in [
        "claude-006-trust.txt",
        "claude-018-trust.txt",
        "claude-030-trust.txt",
        "claude-039-trust.txt",
        "claude-014-startup.txt",
        "claude-015-startup.txt",
    ] {
        expect_dialog(name, &KnownDialog::ClaudeTrust, "No, exit");
    }
}

#[test]
fn ac3_bypass_dialog_defaults_to_no_exit() {
    expect_dialog(
        "claude-050-bypass.txt",
        &KnownDialog::ClaudeBypassConsent,
        "No, exit",
    );
    expect_dialog(
        "claude-051-bypass.txt",
        &KnownDialog::ClaudeBypassConsent,
        "No, exit",
    );
}

#[test]
fn ac4_mcp_dialog_names_the_server() {
    for name in [
        "claude-055-mcpgate-after-trust.txt",
        "claude-056-mcpgate-after-trust.txt",
    ] {
        expect_dialog(
            name,
            &KnownDialog::ClaudeNewMcpServer {
                name: "dummy".to_owned(),
            },
            "Continue without using this MCP server",
        );
    }
    expect_dialog(
        "claude-054-mcpgate-before-trust.txt",
        &KnownDialog::ClaudeTrust,
        "No, exit",
    );
}

#[test]
fn ac5_fullscreen_renderer_dialog() {
    for name in [
        "claude-007-fullscreen.txt",
        "claude-008-fullscreen.txt",
        "claude-009-fullscreen.txt",
        "claude-010-fullscreen.txt",
        "claude-011-fullscreen.txt",
    ] {
        expect_dialog(
            name,
            &KnownDialog::ClaudeFullscreenRenderer,
            "1. Yes, try it",
        );
    }
}

#[test]
fn ac6_ready_and_midturn() {
    for name in [
        "claude-020-ready.txt",
        "claude-021-ready-80x24.txt",
        "claude-022-ready-200x50.txt",
        "claude-023-ready-60x20.txt",
        "claude-024-ready-120x40.txt",
        "claude-025-after-turn.txt",
        "claude-026-seat-ready.txt",
    ] {
        let screen = classify_named(name);
        assert!(screen.is_ready(), "{name} {screen:?}");
    }
    for name in ["claude-040-midturn.txt", "claude-041-midturn.txt"] {
        assert!(matches!(classify_named(name), ClaudeScreen::Busy), "{name}");
    }
}

#[test]
fn ac7_reworded_trust_is_an_unknown_menu() {
    let screen = classify_named("claude-synth-reworded-trust.txt");
    assert!(
        matches!(screen, ClaudeScreen::UnknownMenu { .. }),
        "{screen:?}"
    );
    assert!(screen.requires_attention());
    assert!(!screen.is_ready());
}

proptest! {
    #[test]
    fn text_without_shortcuts_is_not_ready(body in "\\PC{0,180}") {
        prop_assume!(!body.contains("? for shortcuts"));
        prop_assert!(!classify_text(&body).is_ready());
    }
}

fn classify_named(name: &str) -> ClaudeScreen {
    let path = support::fixtures().join("2.1.286").join(name);
    classify_text(&support::read(&path))
}

fn expect_login(name: &str, kind: LoginKind) {
    assert_eq!(classify_named(name), ClaudeScreen::Login(kind), "{name}");
}

fn expect_dialog(name: &str, known: &KnownDialog, cursor: &str) {
    let ClaudeScreen::Dialog(dialog) = classify_named(name) else {
        panic!("{name} is not a dialog");
    };
    assert_eq!(&dialog.known, known, "{name}");
    let at = dialog.cursor.expect("cursor");
    let line = dialog.options.get(at).expect("option");
    assert_eq!(line.text, cursor, "{name}");
}

fn load_labels(dir: &Path) -> BTreeMap<String, Value> {
    let text = support::read(&dir.join("labels.jsonl"));
    let mut labels = BTreeMap::new();
    for line in text.lines() {
        let value: Value = serde_json::from_str(line).expect("label");
        let file = value
            .get("file")
            .and_then(Value::as_str)
            .expect("file")
            .to_owned();
        labels.insert(file, value);
    }
    labels
}

fn assert_label(screen: &ClaudeScreen, label: &Value, name: &str) {
    match label.get("screen").and_then(Value::as_str) {
        Some("ready") => assert!(screen.is_ready(), "{name}"),
        Some("busy") => assert!(matches!(screen, ClaudeScreen::Busy), "{name}"),
        Some("inconclusive") => {
            assert!(matches!(screen, ClaudeScreen::Inconclusive), "{name}");
        }
        Some("unknown_menu") => {
            assert!(matches!(screen, ClaudeScreen::UnknownMenu { .. }), "{name}");
        }
        Some("login") => {
            let kind = match label.get("login").and_then(Value::as_str) {
                Some("theme") => LoginKind::ThemePicker,
                Some("method") => LoginKind::LoginMethod,
                Some("oauth") => LoginKind::OAuthPaste,
                other => panic!("{name} login {other:?}"),
            };
            assert_eq!(screen, &ClaudeScreen::Login(kind), "{name}");
        }
        Some("dialog") => {
            let ClaudeScreen::Dialog(dialog) = screen else {
                panic!("{name} {screen:?}");
            };
            let cursor = label.get("cursor").and_then(Value::as_str).expect("cursor");
            let at = dialog.cursor.expect("cursor index");
            assert_eq!(dialog.options.get(at).expect("line").text, cursor);
            match label.get("dialog").and_then(Value::as_str) {
                Some("trust") => assert_eq!(dialog.known, KnownDialog::ClaudeTrust),
                Some("bypass") => assert_eq!(dialog.known, KnownDialog::ClaudeBypassConsent),
                Some("fullscreen") => {
                    assert_eq!(dialog.known, KnownDialog::ClaudeFullscreenRenderer);
                }
                Some("mcp") => {
                    let mcp = label.get("mcp").and_then(Value::as_str).unwrap_or("dummy");
                    assert_eq!(
                        dialog.known,
                        KnownDialog::ClaudeNewMcpServer {
                            name: mcp.to_owned()
                        }
                    );
                }
                other => panic!("{name} dialog {other:?}"),
            }
        }
        other => panic!("{name} screen {other:?}"),
    }
}
