//! Golden parse of the Claude Code 2.1.286 spike `hooks.jsonl`.
#![expect(clippy::panic, reason = "fixture mismatches should fail the test")]

use std::path::PathBuf;

use crewmux_health::compaction::{
    CompactTrigger, HookEvent, SessionSource, parse_hook_json, parse_hook_value, sha256_hex,
};
use serde_json::Value;

const FILE_SHA: &str = "7c6b2a8b3250087c18633cf74b0a70c50b97b1c0916c393a3c00542e90504fef";

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/hooks/claude/2.1.286")
}

#[test]
fn hook_parse_golden() {
    let dir = fixture();
    let bytes = std::fs::read(dir.join("hooks.jsonl")).expect("hooks");
    assert_eq!(sha256_hex(&bytes), FILE_SHA);
    let body = String::from_utf8(bytes).expect("utf8");
    let labels = std::fs::read_to_string(dir.join("labels.jsonl")).expect("labels");
    let mut count = 0u32;
    let mut session = String::new();
    for (label, line) in labels.lines().zip(body.lines()) {
        let row: Value = serde_json::from_str(label).expect("label");
        let expect = row["sha256"].as_str().expect("sha");
        assert_eq!(sha256_hex(line.as_bytes()), expect, "{label}");
        let wrapper: Value = serde_json::from_str(line).expect("wrapper");
        let input = wrapper.get("input").expect("input");
        assert_eq!(
            wrapper.get("argv_event").and_then(Value::as_str),
            input.get("hook_event_name").and_then(Value::as_str)
        );
        let event = parse_hook_value(input);
        let id = session_of(&event);
        if session.is_empty() {
            session = id.expect("session").to_owned();
        }
        assert_eq!(id, Some(session.as_str()));
        assert_variant(&row, &event);
        count += 1;
    }
    assert_eq!(count, 14);
    assert_eq!(session, "b5461a7f-0588-427a-b7a2-31a9bb2c5b99");
    assert!(parse_hook_json("{").is_err());
}

fn session_of(event: &HookEvent) -> Option<&str> {
    match event {
        HookEvent::SessionStart { session_id, .. }
        | HookEvent::UserPromptSubmit { session_id }
        | HookEvent::PreCompact { session_id, .. }
        | HookEvent::PostCompact { session_id, .. }
        | HookEvent::Stop { session_id }
        | HookEvent::SessionEnd { session_id, .. } => Some(session_id),
        HookEvent::Unknown { .. } => None,
    }
}

fn assert_variant(row: &Value, event: &HookEvent) {
    let name = row["event"].as_str().unwrap_or("");
    let detail = row["detail"].as_str().unwrap_or("");
    match (name, detail, event) {
        (
            "PreCompact",
            "manual",
            HookEvent::PreCompact {
                trigger,
                custom_instructions,
                ..
            },
        ) => {
            assert_eq!(*trigger, CompactTrigger::Manual);
            assert!(
                custom_instructions
                    .as_deref()
                    .unwrap_or("")
                    .contains("OpenRig managed compaction")
            );
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
        ("SessionStart", "startup", HookEvent::SessionStart { source, .. }) => {
            assert_eq!(*source, SessionSource::Startup);
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
            assert_eq!(*context_tokens, Some(32_024));
        }
        (
            "PostCompact",
            "manual",
            HookEvent::PostCompact {
                trigger,
                compact_summary,
                ..
            },
        ) => {
            assert_eq!(*trigger, CompactTrigger::Manual);
            assert!(compact_summary.starts_with("<analysis>"));
        }
        ("UserPromptSubmit", _, HookEvent::UserPromptSubmit { .. })
        | ("Stop", _, HookEvent::Stop { .. }) => {}
        ("SessionEnd", "other", HookEvent::SessionEnd { reason, .. }) => {
            assert_eq!(reason, "other");
        }
        other => panic!("unexpected {other:?}"),
    }
}
