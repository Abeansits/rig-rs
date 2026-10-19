//! Claude Code 2.1.287 panes from PR #18 live runs 2–4.
//!
//! The bytes are the sanitized copies (`/Users/sebas/` rewritten to
//! `/tmp/rig-qa1/`, same length). `SHA256SUMS.sanitized` is the pin.
//! `labels.jsonl` records the raw sha256 when a path was rewritten.
//! Nine frame-recorder copies have no trailing newline, and neither do the
//! two paste payloads.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crewmux_transcript::{fingerprint_raw, unwrap_outermost};
use crewmux_types::{ComposerState, DialogId, ReadinessCode};
use serde_json::Value;

use super::composer::composer_matches;
use super::dialogs::CLAUDE_DIALOGS;
use super::screen::{ClaudeScreen, classify_text};
use super::staged::staged_matches;
use crate::dialog::{AnswerPolicy, EscalateReason, Plan, SeatContext, plan, recognise};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/claude/2.1.287/pr18-run5")
}

fn read(name: &str) -> String {
    let path = dir().join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

/// Frame-recorder copies, plus the two payloads the harness wrote without a final newline.
const NO_NEWLINE: &[&str] = &[
    "a02p-paste-payload-12-lines.txt",
    "a03p-paste-payload-1-line.txt",
    "a05-paste-bare-no-linecount-run4.txt",
    "a06-paste-envelope-plus5-lines.txt",
    "a07-paste-envelope-plus5-lines-run3.txt",
    "b04-trust-cursor-yes-rig-driver-run3.txt",
    "b05-trust-cursor-no-rig-run4.txt",
    "b06-trust-cursor-yes-rig-run4.txt",
    "c01-restore-pasted-plus10-lines.txt",
    "e11-pane-compacting.txt",
    "f01-ready-idle-try-placeholder.txt",
];

const READY: &[&str] = &[
    "b07-ready-after-trust-probe.txt",
    "c05-after-permission-esc.txt",
    "e12-pane-after-compact-done.txt",
    "e13-pane-send-failed-draft-x.txt",
    "f01-ready-idle-try-placeholder.txt",
    "f02-ready-idle-auto-mode-banner.txt",
    "f03-ready-idle-empty-after-turn.txt",
];

#[test]
fn every_fixture_matches_the_sanitized_checksum() {
    let pins = checksums();
    assert_eq!(pins.len(), 54);
    for (name, sha) in &pins {
        let text = read(name);
        assert_eq!(fingerprint_raw(&text), *sha, "{name}");
        assert!(!text.contains("/Users/"), "{name}");
        assert_eq!(
            text.ends_with('\n'),
            !NO_NEWLINE.contains(&name.as_str()),
            "{name} trailing newline"
        );
    }
    let mut labels = 0u32;
    let mut rewritten = 0u32;
    for line in read("labels.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap_or_else(|err| panic!("{err}"));
        let name = row["file"].as_str().unwrap_or_else(|| panic!("file"));
        let sha = row["sha256"].as_str().unwrap_or_else(|| panic!("sha"));
        assert_eq!(pins.get(name).map(String::as_str), Some(sha), "{name}");
        if let Some(raw) = row.get("raw_sha256").and_then(Value::as_str) {
            assert_ne!(raw, sha, "{name}");
            assert_eq!(
                row["path_rewritten"].as_str(),
                Some("/Users/sebas/ -> /tmp/rig-qa1/")
            );
            rewritten += 1;
        }
        labels += 1;
    }
    assert_eq!(labels, 54);
    assert_eq!(rewritten, 31);
}

#[test]
fn paste_markers_match_on_the_compact_path() {
    let plus11 = read("a02p-paste-payload-12-lines.txt");
    assert_eq!(plus11.matches('\n').count(), 11);
    expect_paste("a01-paste-plus11-lines.txt", &plus11);
    expect_paste("a02-paste-plus11-lines-1800ms.txt", &plus11);
    let literal = read("a03p-paste-payload-1-line.txt");
    assert_eq!(literal.len(), 31);
    expect_paste("a03-paste-single-line-literal.txt", &literal);

    let bare = pasted_inner("g-run4A-L033-prep.jsonl");
    assert!(!bare.contains('\n') && bare.len() > 31);
    expect_paste("a04-paste-bare-no-linecount-run2.txt", &bare);
    expect_paste("a05-paste-bare-no-linecount-run4.txt", &bare);
    let bare_screen = read("a04-paste-bare-no-linecount-run2.txt");
    assert!(!staged_matches(&bare_screen, &literal));
    assert!(!composer_matches(&bare_screen, &plus11));

    let plus5 = pasted_inner("g-run4A-L005-send-envelope.jsonl");
    assert_eq!(plus5.matches('\n').count(), 5);
    expect_paste("a06-paste-envelope-plus5-lines.txt", &plus5);
    expect_paste("a07-paste-envelope-plus5-lines-run3.txt", &plus5);
    let fewer = drop_last_line(&plus5);
    let envelope = read("a06-paste-envelope-plus5-lines.txt");
    assert!(!staged_matches(&envelope, &fewer));
    assert!(!composer_matches(&envelope, &fewer));

    let plus10 = pasted_inner("g-run4A-L060-restore.jsonl");
    assert_eq!(plus10.matches('\n').count(), 10);
    expect_paste("c01-restore-pasted-plus10-lines.txt", &plus10);
}

#[test]
fn trust_dialogs_read_the_cursor_and_plan_yes_for_the_folder() {
    for name in [
        "b01-trust-cursor-no.txt",
        "b01-trust-cursor-no.escapes.txt",
        "b02-trust-cursor-no-probe.txt",
        "b05-trust-cursor-no-rig-run4.txt",
    ] {
        expect_trust(name, "No, exit");
    }
    for name in [
        "b03-trust-cursor-yes-settled.txt",
        "b04-trust-cursor-yes-rig-driver-run3.txt",
        "b06-trust-cursor-yes-rig-run4.txt",
    ] {
        expect_trust(name, "Yes, I trust this folder");
    }
}

#[test]
fn permission_dialogs_are_attention_and_are_not_answered() {
    for name in [
        "c02-permission-read-outside-workdirs.txt",
        "c03-permission-read-outside-poll.txt",
        "c04-permission-read-outside-rigcapture.txt",
    ] {
        let text = read(name);
        let screen = classify_text(&text);
        assert!(screen.requires_attention(), "{name}");
        assert_eq!(screen.code(), Some(ReadinessCode::UnknownMenu), "{name}");
        assert!(recognise(CLAUDE_DIALOGS, &text).is_none(), "{name}");
    }
}

#[test]
fn nocolor_suggestions_stay_prompt_draft() {
    for name in [
        "d01-suggestion-nocolor-run3.txt",
        "d01-suggestion-nocolor-run3.escapes.txt",
        "d02-suggestion-nocolor-run4.txt",
        "d03-suggestion-nocolor-after-recall.txt",
    ] {
        let text = read(name);
        assert!(
            !text.contains('\u{1b}'),
            "{name} has no SGR; a faint suggestion is not in this set"
        );
        let screen = classify_text(&text);
        assert_eq!(screen.code(), Some(ReadinessCode::PromptDraft), "{name}");
    }
}

#[test]
fn idle_busy_and_typed_draft_panes() {
    for name in READY {
        assert_eq!(
            classify_text(&read(name)),
            ClaudeScreen::Ready(ComposerState { bordered: true }),
            "{name}"
        );
    }
    assert_eq!(
        classify_text(&read("e11-pane-compacting.txt")),
        ClaudeScreen::Busy
    );
    assert_eq!(
        classify_text(&read("e14-pane-x-draft.txt")).code(),
        Some(ReadinessCode::PromptDraft)
    );
}

fn expect_paste(name: &str, payload: &str) {
    let screen = read(name);
    assert!(
        matches!(classify_text(&screen), ClaudeScreen::Draft),
        "{name} holds the paste"
    );
    assert!(staged_matches(&screen, payload), "{name}");
    assert!(composer_matches(&screen, payload), "{name}");
}

fn expect_trust(name: &str, cursor: &str) {
    let text = read(name);
    let screen = classify_text(&text);
    let ClaudeScreen::Dialog(dialog) = &screen else {
        panic!("{name}: {screen:?}");
    };
    assert_eq!(dialog.id, DialogId::ClaudeTrust, "{name}");
    let at = dialog.cursor.unwrap_or_else(|| panic!("{name} cursor"));
    let label = dialog
        .options
        .get(at)
        .unwrap_or_else(|| panic!("{name} option"))
        .label
        .as_str();
    assert_eq!(label, cursor, "{name}");
    let found = recognise(CLAUDE_DIALOGS, &text).unwrap_or_else(|| panic!("{name}"));
    let folder = found
        .params
        .folder
        .clone()
        .unwrap_or_else(|| panic!("{name} folder"));
    let home = PathBuf::from("/tmp/rig-qa1");
    let policy = AnswerPolicy {
        auto_trust: true,
        auto_approve_rig_tools: false,
    };
    let matched = SeatContext {
        cwd: &folder,
        home: &home,
    };
    assert_eq!(
        plan(&found, policy, &matched),
        Plan::Answer("Yes, I trust this folder"),
        "{name}"
    );
    let other = PathBuf::from("/tmp/not-the-folder");
    let refused = SeatContext {
        cwd: &other,
        home: &home,
    };
    assert_eq!(
        plan(&found, policy, &refused),
        Plan::Escalate(EscalateReason::Policy),
        "{name}"
    );
}

fn pasted_inner(name: &str) -> String {
    let value: Value =
        serde_json::from_str(&read(name)).unwrap_or_else(|err| panic!("{name}: {err}"));
    let content = value
        .pointer("/message/content")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{name} content"));
    unwrap_outermost(content).unwrap_or_else(|| panic!("{name} wrapper"))
}

fn drop_last_line(payload: &str) -> String {
    let cut = payload.rfind('\n').unwrap_or_else(|| panic!("newline"));
    payload.get(..cut).unwrap_or(payload).to_owned()
}

fn checksums() -> BTreeMap<String, String> {
    let mut pins = BTreeMap::new();
    for line in read("SHA256SUMS.sanitized").lines() {
        let (sha, name) = line
            .split_once("  ")
            .unwrap_or_else(|| panic!("checksum {line}"));
        pins.insert(name.to_owned(), sha.to_owned());
    }
    pins
}
