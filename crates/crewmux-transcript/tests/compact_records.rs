//! Compact records are not user turns.
//!
//! The attached spike has no Claude transcript jsonl. `comp.py` projected
//! `compact_boundary_rows` in `s2-result.json` down to `type`, `subtype`,
//! `compactMetadata`, `isCompactSummary`, and `sessionId`. Those two objects
//! are copied below. The content-bearing lines are not spike bytes: the
//! archive does not include the jsonl they were projected from. Skips follow
//! record structure (`isCompactSummary`, `isMeta`, `subtype`, or a command-tag
//! envelope), so a user message that merely quotes `<local-command-stdout>`
//! still counts.
#![expect(
    clippy::expect_used,
    reason = "a temp-dir or fixture failure should fail the test"
)]

use std::path::PathBuf;

use crewmux_transcript::{
    ClaudeTranscript, Delivery, TranscriptCursor, user_turns_after, verify_delivery,
};
use serde_json::{Value, json};

const SESSION: &str = "b5461a7f-0588-427a-b7a2-31a9bb2c5b99";

const PROJECTED: &str = r#"{"compactMetadata":{"cumulativeDroppedTokens":30792,"durationMs":12993,"postTokens":2683,"preTokens":33475,"preservedMessages":{"allUuids":["4fb022bf-6b8a-4fb6-aa90-ab9124b4a6a7","b2ad90ba-472d-423f-928f-3dbb40bc4251","ca48c8e5-9377-473c-93ea-374eb3febd1f","6dcd5b42-2bad-4c5f-a5f8-dea0829ee7f2","22f031ef-c9f1-4873-b1ab-91f2f7535f00"],"anchorUuid":"cde3eff0-22eb-457c-9fbc-e506bcfe8daf","uuids":["4fb022bf-6b8a-4fb6-aa90-ab9124b4a6a7","b2ad90ba-472d-423f-928f-3dbb40bc4251","6dcd5b42-2bad-4c5f-a5f8-dea0829ee7f2","22f031ef-c9f1-4873-b1ab-91f2f7535f00"]},"preservedSegment":{"anchorUuid":"cde3eff0-22eb-457c-9fbc-e506bcfe8daf","headUuid":"4fb022bf-6b8a-4fb6-aa90-ab9124b4a6a7","tailUuid":"22f031ef-c9f1-4873-b1ab-91f2f7535f00"},"trigger":"manual"},"isCompactSummary":null,"sessionId":"b5461a7f-0588-427a-b7a2-31a9bb2c5b99","subtype":"compact_boundary","type":"system"}
{"compactMetadata":null,"isCompactSummary":true,"sessionId":"b5461a7f-0588-427a-b7a2-31a9bb2c5b99","subtype":null,"type":"user"}
"#;

#[test]
fn projected_spike_rows_are_not_user_turns() {
    let root = scratch("projected");
    let path = write_transcript(&root, PROJECTED);
    let turns = user_turns_after(&path, 0).expect("turns");
    assert_eq!(turns.len(), 0);
    drop(std::fs::remove_dir_all(&root));
}

#[test]
fn compact_records_do_not_hide_a_real_user_turn() {
    let root = scratch("mixed");
    let sent = "S2T3 Restore check after compaction: what is the codeword?";
    let quoted = "notes mention <local-command-stdout>compacted</local-command-stdout> in prose";
    let only_tag = "<local-command-stdout>compacted</local-command-stdout>";
    let body = format!(
        "{PROJECTED}{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        user(sent, &[]),
        user(
            "summary that must not count",
            &[("isCompactSummary", json!(true))]
        ),
        user("meta that must not count", &[("isMeta", json!(true))]),
        user(
            "stdout that must not count",
            &[("subtype", json!("local_command"))]
        ),
        user(quoted, &[]),
        user(only_tag, &[]),
        user(
            "<command-name>/compact</command-name>\n<command-args>keep</command-args>",
            &[]
        ),
    );
    let path = write_transcript(&root, &body);
    let turns = user_turns_after(&path, 0).expect("turns");
    let texts: Vec<&str> = turns.iter().map(|turn| turn.text.as_str()).collect();
    assert_eq!(texts, vec![sent, quoted, only_tag]);
    drop(std::fs::remove_dir_all(&root));
}

#[test]
fn quoted_stdout_tag_still_confirms() {
    let root = scratch("quoted");
    let sent = "<local-command-stdout>compacted</local-command-stdout>";
    let path = write_transcript(&root, &format!("{}\n", user(sent, &[])));
    assert!(path.is_file());
    let source =
        ClaudeTranscript::open(&root, PathBuf::from("/tmp/work").as_path(), SESSION).expect("open");
    let delivery = verify_delivery(&source, TranscriptCursor(0), sent).expect("verify");
    assert_eq!(delivery, Delivery::Landed);
    drop(std::fs::remove_dir_all(&root));
}

fn user(content: &str, extra: &[(&str, Value)]) -> String {
    let mut value = json!({
        "type": "user",
        "message": {"role": "user", "content": content}
    });
    let Some(object) = value.as_object_mut() else {
        return value.to_string();
    };
    for (key, item) in extra {
        object.insert((*key).to_owned(), item.clone());
    }
    value.to_string()
}

fn write_transcript(root: &std::path::Path, body: &str) -> PathBuf {
    let cwd = PathBuf::from("/tmp/work");
    let path = crewmux_transcript::transcript_path(root, &cwd, SESSION).expect("path");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    std::fs::write(&path, body).expect("write");
    path
}

fn scratch(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "rig-transcript-compact-{label}-{}",
        std::process::id()
    ));
    drop(std::fs::remove_dir_all(&root));
    std::fs::create_dir_all(&root).expect("scratch");
    root
}
