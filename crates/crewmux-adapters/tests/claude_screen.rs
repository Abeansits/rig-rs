//! Claude screen corpus.
//!
//! `2.1.286` and `2.1.289` are byte copies of the spike captures. `2.1.287`
//! holds the 2026-10-18 Claude Code 2.1.287 panes: the trust cursor starts on
//! "No, exit" and the folder is one line. The permission dialog and the
//! no-color suggestion are from the same day's compact run, already rewritten
//! to `/tmp/rig-qa1/`. `claude-trust.txt` is synthetic (cursor already on Yes,
//! hard-wrapped folder, U+FFFD). Sanitized copies rewrite `/Users/sebas/` to
//! `/tmp/rig-qa1/`; `labels.jsonl` records both hashes.
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "a mismatched fixture should fail the test"
)]

use std::collections::BTreeMap;
use std::path::Path;

use crewmux_adapters::claude::{ClassifiedDialog, ClaudeScreen, LoginKind, classify_text};
use crewmux_types::DialogId;
use proptest::prelude::*;
use serde_json::Value;

#[path = "common/mod.rs"]
mod support;

#[test]
fn every_corpus_file_is_labelled() {
    for version in ["2.1.286", "2.1.287", "2.1.289"] {
        check_corpus(&support::fixtures().join(version));
    }
}

fn check_corpus(dir: &Path) {
    let labels = load_labels(dir);
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).expect("corpus") {
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
        let text = support::read(&path);
        let sha = crewmux_transcript::fingerprint_raw(&text);
        assert_eq!(
            label.get("sha256").and_then(Value::as_str),
            Some(sha.as_str()),
            "{name} sha"
        );
        let screen = classify_text(&text);
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
    ] {
        expect_dialog(name, DialogId::ClaudeTrust, "No, exit");
    }
}

#[test]
fn ac3_bypass_dialog_defaults_to_no_exit() {
    expect_dialog(
        "claude-050-bypass-after-trust.txt",
        DialogId::ClaudeBypassConsent,
        "No, exit",
    );
    expect_dialog(
        "claude-051-bypass-after-trust.txt",
        DialogId::ClaudeBypassConsent,
        "No, exit",
    );
}

#[test]
fn ac4_mcp_dialog_names_the_server() {
    for name in [
        "claude-055-mcpgate-after-trust.txt",
        "claude-056-mcpgate-after-trust.txt",
    ] {
        let ClaudeScreen::Dialog(dialog) = classify_named(name) else {
            panic!("{name} is not a dialog");
        };
        assert_eq!(dialog.id, DialogId::ClaudeNewMcpServer, "{name}");
        assert_eq!(dialog.server.as_deref(), Some("dummy"), "{name}");
        let at = dialog.cursor.expect("cursor");
        assert_eq!(
            dialog.options.get(at).expect("option").label,
            "Continue without using this MCP server",
            "{name}"
        );
    }
    expect_dialog("claude-054-mcpgate.txt", DialogId::ClaudeTrust, "No, exit");
}

#[test]
fn ac5_fullscreen_renderer_dialog() {
    for name in [
        "claude-007-startup.txt",
        "claude-008-stable-unclassified.txt",
        "claude-009-resize80x24.txt",
        "claude-010-resize200x50.txt",
        "claude-011-resize60x20.txt",
        "claude-012-resize120x40.txt",
        "claude-013-midturn.txt",
    ] {
        expect_dialog(name, DialogId::ClaudeFullscreenRenderer, "Yes, try it");
    }
}

#[test]
fn ac6_ready_and_midturn() {
    for name in [
        "claude-020-ready.txt",
        "claude-021-resize80x24.txt",
        "claude-022-resize200x50.txt",
        "claude-023-resize60x20.txt",
        "claude-024-resize120x40.txt",
        "claude-026-midturn.txt",
        "claude-032-ready.txt",
        "claude-041-ready.txt",
    ] {
        let screen = classify_named(name);
        assert!(screen.is_ready(), "{name} {screen:?}");
    }
    assert!(
        matches!(classify_named("claude-025-midturn.txt"), ClaudeScreen::Busy),
        "midturn"
    );
}

#[test]
fn ac7_mcp_list_is_an_unknown_menu() {
    let screen = classify_named("claude-064-s6-mcp-list.txt");
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
        // Faint SGR is a suggestion (PE-026). Without an escape it stays a draft.
        prop_assume!(!body.contains('\u{1b}'));
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

fn expect_dialog(name: &str, id: DialogId, cursor: &str) {
    let ClaudeScreen::Dialog(dialog) = classify_named(name) else {
        panic!("{name} is not a dialog");
    };
    assert_eq!(dialog.id, id, "{name}");
    let at = dialog.cursor.expect("cursor");
    let line = dialog.options.get(at).expect("option");
    assert_eq!(line.label, cursor, "{name}");
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
        Some("ready") => assert!(screen.is_ready(), "{name} {screen:?}"),
        Some("draft") => {
            assert!(matches!(screen, ClaudeScreen::Draft), "{name} {screen:?}");
            assert!(!screen.is_ready(), "{name}");
        }
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
            assert_dialog(dialog, label, name);
        }
        other => panic!("{name} screen {other:?}"),
    }
}

#[test]
fn no_color_suggestion_is_a_draft_and_dim_sgr_is_ready() {
    let path = support::fixtures()
        .join("2.1.287")
        .join("claude-2.1.287-suggestion-no-color.txt");
    let text = support::read(&path);
    assert!(
        !text.contains('\u{1b}'),
        "the live NO_COLOR capture has no SGR"
    );
    assert!(
        matches!(classify_text(&text), ClaudeScreen::Draft),
        "a no-color suggestion is the same bytes as a typed draft"
    );
    let dimmed = faint_suggestion(&text);
    assert!(
        classify_text(&dimmed).is_ready(),
        "faint SGR is a suggestion, not a draft: {dimmed}"
    );
}

#[test]
fn tool_approval_is_an_unknown_menu() {
    let screen = "Do you want to proceed?\n ❯ 1. Yes\n   2. No\n";
    assert!(
        matches!(classify_text(screen), ClaudeScreen::UnknownMenu { .. }),
        "{screen}"
    );
}

fn faint_suggestion(text: &str) -> String {
    let phrase = "allow reading the marker path";
    let mut out = String::new();
    for line in text.lines() {
        if !out.is_empty() {
            out.push('\n');
        }
        let Some(start) = line.find(phrase) else {
            out.push_str(line);
            continue;
        };
        let head = line.get(..start).unwrap_or("");
        let rest = line.get(start.saturating_add(phrase.len())..).unwrap_or("");
        out.push_str(head);
        out.push_str("\u{1b}[2m");
        out.push_str(phrase);
        out.push_str("\u{1b}[22m");
        out.push_str(rest);
    }
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

#[test]
fn scrollback_interrupt_is_not_the_live_region() {
    let rule = "─".repeat(40);
    let scrollback =
        format!("esc to interrupt\nold answer\n{rule}\n❯ \n{rule}\n  ? for shortcuts\n");
    assert!(classify_text(&scrollback).is_ready(), "{scrollback}");
    let live = format!("{rule}\n❯ \n{rule}\n  esc to interrupt\n");
    assert!(matches!(classify_text(&live), ClaudeScreen::Busy), "{live}");
}

fn assert_dialog(dialog: &ClassifiedDialog, label: &Value, name: &str) {
    let cursor = label.get("cursor").and_then(Value::as_str).expect("cursor");
    let at = dialog.cursor.expect("cursor index");
    assert_eq!(dialog.options.get(at).expect("line").label, cursor);
    match label.get("dialog").and_then(Value::as_str) {
        Some("trust") => assert_eq!(dialog.id, DialogId::ClaudeTrust),
        Some("bypass") => assert_eq!(dialog.id, DialogId::ClaudeBypassConsent),
        Some("fullscreen") => assert_eq!(dialog.id, DialogId::ClaudeFullscreenRenderer),
        Some("mcp") => {
            let mcp = label.get("mcp").and_then(Value::as_str).unwrap_or("dummy");
            assert_eq!(dialog.id, DialogId::ClaudeNewMcpServer);
            assert_eq!(dialog.server.as_deref(), Some(mcp));
        }
        other => panic!("{name} dialog {other:?}"),
    }
}
