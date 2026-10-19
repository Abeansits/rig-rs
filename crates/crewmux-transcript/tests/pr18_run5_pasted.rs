//! `<pasted_content id=…>` lines from the Claude Code 2.1.287 compact runs.
//!
//! The id is reused, so confirmation is the inner text.
#![expect(
    clippy::expect_used,
    reason = "a fixture that is not one user turn should fail the test"
)]

use std::path::PathBuf;

use crewmux_transcript::{TurnMatch, fingerprint_raw, match_turn, unwrap_outermost};
use crewmux_types::RuntimeKind;
use serde_json::Value;

const LINES: &[&str] = &[
    "g-run3-L005-prep.jsonl",
    "g-run3-L053-restore.jsonl",
    "g-run3-L081-restore.jsonl",
    "g-run4A-L005-send-envelope.jsonl",
    "g-run4A-L033-prep.jsonl",
    "g-run4A-L060-restore.jsonl",
    "g-run4A-L086-send-envelope.jsonl",
    "g-run4A-L131-restore.jsonl",
    "g-run4E-L005-prep.jsonl",
    "g-run4E-L053-restore.jsonl",
];

fn read(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/claude/2.1.287/pr18-run5")
        .join(name);
    std::fs::read_to_string(&path).expect("fixture")
}

#[test]
fn each_pasted_content_line_confirms_its_inner_text() {
    let pins = checksums();
    assert_eq!(LINES.len(), 10);
    for name in LINES {
        let raw = read(name);
        assert_eq!(fingerprint_raw(&raw), pins[*name], "{name}");
        assert!(raw.ends_with('\n'), "{name}");
        let content = content(&raw, name);
        assert!(content.contains("<pasted_content id="), "{name}");
        let inner = unwrap_outermost(&content).expect(name);
        assert_eq!(
            match_turn(RuntimeKind::ClaudeCode, &inner, &content),
            TurnMatch::Equal,
            "{name}"
        );
        assert_eq!(
            match_turn(RuntimeKind::ClaudeCode, "not the paste", &content),
            TurnMatch::Different,
            "{name}"
        );
    }
}

#[test]
fn a_stray_key_after_the_prep_wrapper_is_not_confirmed() {
    let raw = read("g-run4A-L099-prep.jsonl");
    let stored = content(&raw, "g-run4A-L099-prep.jsonl");
    assert!(stored.ends_with("</pasted_content id=\"efd8\">\n\nx"));
    assert!(unwrap_outermost(&stored).is_none());
    let cleaned = stored.trim_end_matches('x').trim_end();
    let inner = unwrap_outermost(cleaned).expect("wrapper");
    assert_eq!(
        match_turn(RuntimeKind::ClaudeCode, &inner, cleaned),
        TurnMatch::Equal
    );
    assert_eq!(
        match_turn(RuntimeKind::ClaudeCode, &inner, &stored),
        TurnMatch::Different
    );
}

#[test]
fn a_reused_paste_id_does_not_confirm_a_different_turn() {
    let prep = content(&read("g-run3-L005-prep.jsonl"), "prep");
    let restore = content(&read("g-run3-L053-restore.jsonl"), "restore");
    assert!(prep.contains("id=\"f360\"") && restore.contains("id=\"f360\""));
    let prep_inner = unwrap_outermost(&prep).expect("prep wrapper");
    assert_eq!(
        match_turn(RuntimeKind::ClaudeCode, &prep_inner, &restore),
        TurnMatch::Different
    );
    let later = content(&read("g-run4A-L033-prep.jsonl"), "later prep");
    let envelope = content(&read("g-run4A-L005-send-envelope.jsonl"), "envelope");
    assert!(later.contains("id=\"efd8\"") && envelope.contains("id=\"efd8\""));
    let later_inner = unwrap_outermost(&later).expect("later wrapper");
    assert_eq!(
        match_turn(RuntimeKind::ClaudeCode, &later_inner, &envelope),
        TurnMatch::Different
    );
}

fn content(raw: &str, name: &str) -> String {
    let value: Value = serde_json::from_str(raw).expect(name);
    value
        .pointer("/message/content")
        .and_then(Value::as_str)
        .expect(name)
        .to_owned()
}

fn checksums() -> std::collections::BTreeMap<String, String> {
    let mut pins = std::collections::BTreeMap::new();
    for line in read("SHA256SUMS.sanitized").lines() {
        let (sha, name) = line.split_once("  ").expect("checksum");
        pins.insert(name.to_owned(), sha.to_owned());
    }
    pins
}
