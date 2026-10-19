//! Claude Code 2.1.287 stores a pasted turn as `<pasted_content id="…">`.
//!
//! The id is reused across pastes, so a match is the inner text. These rows
//! are the prep and restore user turns from the 2026-10-18 compact run, with
//! paths already rewritten to `/tmp/rig-qa1/`.
#![expect(
    clippy::expect_used,
    reason = "a fixture that is not one user turn should fail the test"
)]

use crewmux_transcript::{TurnMatch, match_turn};
use crewmux_types::RuntimeKind;
use serde_json::Value;

#[test]
fn pasted_content_matches_the_inner_text_not_the_paste_id() {
    let prep = turn(include_str!(
        "fixtures/transcripts/claude/2.1.287/prep-pasted.jsonl"
    ));
    let restore = turn(include_str!(
        "fixtures/transcripts/claude/2.1.287/restore-pasted.jsonl"
    ));
    let prep_sent = include_str!("fixtures/transcripts/claude/2.1.287/prep-pasted.sent.txt");
    let restore_sent = include_str!("fixtures/transcripts/claude/2.1.287/restore-pasted.sent.txt");
    assert!(prep.contains("</pasted_content id=\"f360\">"));
    assert!(restore.contains("</pasted_content id=\"f360\">"));
    assert_eq!(
        match_turn(RuntimeKind::ClaudeCode, prep_sent, &prep),
        TurnMatch::Equal
    );
    assert_eq!(
        match_turn(RuntimeKind::ClaudeCode, restore_sent, &restore),
        TurnMatch::Equal
    );
    assert_eq!(
        match_turn(RuntimeKind::ClaudeCode, prep_sent, &restore),
        TurnMatch::Different
    );
}

fn turn(line: &str) -> String {
    let value: Value = serde_json::from_str(line).expect("jsonl");
    value
        .pointer("/message/content")
        .and_then(Value::as_str)
        .expect("user content")
        .to_owned()
}
