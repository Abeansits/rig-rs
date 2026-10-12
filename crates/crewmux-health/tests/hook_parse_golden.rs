//! Golden parse of the documented Claude 2.1.286 hook inputs.
#![expect(clippy::panic, reason = "fixture mismatches should fail the test")]

use std::path::PathBuf;

use crewmux_health::compaction::{
    CompactTrigger, HookEvent, SessionSource, parse_hook_json, sha256_hex,
};
use serde_json::Value;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/hooks/claude/2.1.286")
}

#[test]
fn hook_parse_golden() {
    let dir = fixture();
    let body = std::fs::read_to_string(dir.join("hooks.jsonl")).expect("hooks");
    let labels = std::fs::read_to_string(dir.join("labels.jsonl")).expect("labels");
    let mut count = 0u32;
    for (label, line) in labels.lines().zip(body.lines()) {
        let row: Value = serde_json::from_str(label).expect("label");
        let expect = row["sha256"].as_str().expect("sha");
        assert_eq!(sha256_hex(line.as_bytes()), expect, "{label}");
        let event = parse_hook_json(line).expect("json");
        assert_variant(&row, &event);
        count += 1;
    }
    assert_eq!(count, 10);
    assert!(parse_hook_json("{").is_err());
}

fn assert_variant(row: &Value, event: &HookEvent) {
    let name = row["event"].as_str().unwrap_or("");
    let detail = row["detail"].as_str().unwrap_or("");
    match (name, detail, event) {
        ("PreCompact", "manual" | "manual-extra", HookEvent::PreCompact { trigger, .. }) => {
            assert_eq!(*trigger, CompactTrigger::Manual);
        }
        (
            "PreCompact",
            "auto",
            HookEvent::PreCompact {
                trigger,
                custom_instructions,
                ..
            },
        ) => {
            assert_eq!(*trigger, CompactTrigger::Auto);
            assert!(custom_instructions.is_none());
        }
        ("SessionStart", "compact", HookEvent::SessionStart { source, .. }) => {
            assert_eq!(*source, SessionSource::Compact);
        }
        (
            "SessionStart",
            "resume",
            HookEvent::SessionStart {
                source,
                context_tokens,
                ..
            },
        ) => {
            assert_eq!(*source, SessionSource::Resume);
            assert_eq!(*context_tokens, Some(182_340));
        }
        (
            "PostCompact",
            _,
            HookEvent::PostCompact {
                compact_summary, ..
            },
        ) => {
            assert_eq!(compact_summary, "Summary of the compacted conversation...");
        }
        ("UserPromptSubmit", _, HookEvent::UserPromptSubmit { session_id }) => {
            assert_eq!(session_id, "abc123");
        }
        ("Stop", _, HookEvent::Stop { session_id }) => assert_eq!(session_id, "abc123"),
        ("SessionEnd", _, HookEvent::SessionEnd { reason, .. }) => assert_eq!(reason, "other"),
        ("Elicitation", _, HookEvent::Unknown { name, .. }) => assert_eq!(name, "Elicitation"),
        other => panic!("unexpected {other:?}"),
    }
}
