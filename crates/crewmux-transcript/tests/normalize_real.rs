//! ADR-0009 against reconstructed Claude and Codex rows.
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "a mismatched fixture should fail the test"
)]
//!
//! The JSON files are built from the amendment's invariants. They are not the
//! original Mac payload bytes.

use std::path::PathBuf;

use crewmux_transcript::{TurnMatch, fingerprint, fingerprint_raw, match_turn, normalize_for};
use crewmux_types::RuntimeKind;
use serde_json::Value;

#[test]
fn claude_rows_match_after_unwrap_and_tab_expansion() {
    let doc = load("claude/s1-claude.json");
    let rows = doc.get("rows").and_then(Value::as_array).expect("rows");
    let mut single = 0;
    let mut multi = 0;
    for row in rows {
        let sent = row.get("sent").and_then(Value::as_str).expect("sent");
        let stored = row.get("stored").and_then(Value::as_str).expect("stored");
        assert_eq!(
            fingerprint(RuntimeKind::ClaudeCode, sent),
            fingerprint(RuntimeKind::ClaudeCode, stored),
            "{}",
            row.get("name").and_then(Value::as_str).unwrap_or("row")
        );
        assert_eq!(
            match_turn(RuntimeKind::ClaudeCode, sent, stored),
            TurnMatch::Equal
        );
        match row.get("kind").and_then(Value::as_str) {
            Some("single") => {
                single += 1;
                assert_eq!(sent, stored);
                assert_eq!(fingerprint_raw(sent), fingerprint_raw(stored));
            }
            Some("multi") => {
                multi += 1;
                assert_ne!(fingerprint_raw(sent), fingerprint_raw(stored));
                assert_ne!(sent, stored);
            }
            other => panic!("kind {other:?}"),
        }
    }
    assert_eq!(single, 10);
    assert_eq!(multi, 15);
    let tabs = rows
        .iter()
        .find(|row| row.get("name").and_then(Value::as_str) == Some("tabs-49"))
        .expect("tabs");
    let sent = tabs.get("sent").and_then(Value::as_str).expect("sent");
    let stored = tabs.get("stored").and_then(Value::as_str).expect("stored");
    assert_eq!(sent.len(), 2409);
    assert_eq!(sent.chars().filter(|ch| *ch == '\t').count(), 49);
    assert_eq!(stored.len(), 2614);
    let wide = rows
        .iter()
        .find(|row| row.get("name").and_then(Value::as_str) == Some("wide-64k"))
        .expect("wide");
    let sent = wide.get("sent").and_then(Value::as_str).expect("sent");
    let stored = wide.get("stored").and_then(Value::as_str).expect("stored");
    assert_eq!(sent.len(), 65_536);
    assert_eq!(sent.chars().filter(|ch| *ch == '\t').count(), 0);
    assert_eq!(stored.len(), 65_594);
}

#[test]
fn codex_rows_stay_byte_exact() {
    let doc = load("codex/s1-codex.json");
    let rows = doc.get("rows").and_then(Value::as_array).expect("rows");
    assert_eq!(rows.len(), 25);
    for row in rows {
        let sent = row.get("sent").and_then(Value::as_str).expect("sent");
        let stored = row.get("stored").and_then(Value::as_str).expect("stored");
        assert_eq!(sent, stored);
        assert_eq!(fingerprint_raw(sent), fingerprint_raw(stored));
        assert_eq!(
            fingerprint(RuntimeKind::Codex, sent),
            fingerprint(RuntimeKind::Codex, stored)
        );
        assert_eq!(
            normalize_for(RuntimeKind::Codex, sent),
            normalize_for(RuntimeKind::Codex, stored)
        );
    }
    assert_eq!(
        match_turn(RuntimeKind::Codex, "a\tb", "a    b"),
        TurnMatch::Different
    );
    assert_eq!(
        match_turn(RuntimeKind::ClaudeCode, "a\tb", "a    b"),
        TurnMatch::Equal
    );
}

#[test]
fn two_wrappers_are_composite() {
    let stored = "\n<pasted_content id=\"a\">\none\n</pasted_content>\n<pasted_content id=\"b\">\ntwo\n</pasted_content>\n";
    assert_eq!(
        match_turn(RuntimeKind::ClaudeCode, "one", stored),
        TurnMatch::Composite
    );
}

fn load(rel: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/transcripts")
        .join(rel);
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    serde_json::from_str(&text).expect("json")
}
