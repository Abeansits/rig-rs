//! Restore prompt golden. Plugin skills this build does not ship are absent.
//!
//! The marker JSON is in the prompt (PE-025). The text does not name a path
//! outside the seat.

use std::path::Path;

use crewmux_health::compaction::{
    RESTORE_INLINE_LIMIT, RestorePrompt, build_post_compact_restore_prompt,
};

const JSON: &str = "{\"attempt\":\"a\"}";

const GOLDEN: &str = "Please respond to this normal user message now by restoring this Claude session after compaction. This is the operator-authorized OpenRig restore request referenced by the compact summary; it is not local-command stdout or hook output. Restoration is the current task. Do not wait for a future user request or task assignment before reading the required files. If you did not compact (your earlier context is still present), say so and skip the restore reading. The pending restore marker is included below. Do not read a file outside this working directory to find it. Restore marker JSON: {\"attempt\":\"a\"} If the marker is missing and you have no restore map, rebuild a packet from this Claude JSONL transcript: /tmp/t.jsonl. Follow the marker's restoreInstruction and postCompactInstruction when present, read your newest restore map and use the restore packet as a lookup, then reply with: restored from packet at <path>; resumed at step <X>.";

#[test]
fn restore_prompt_golden() {
    let prompt = build_post_compact_restore_prompt(&input(JSON)).expect("prompt");
    assert_eq!(prompt, GOLDEN);
    assert!(prompt.contains(JSON));
    assert!(!prompt.contains("restore-pending"));
    assert!(!prompt.contains("/marks/"));
    assert!(!prompt.contains("First, look for the pending restore marker"));
    assert!(!prompt.contains("SKILL.md"));
    assert!(!prompt.contains("refocusing"));
    assert!(!prompt.contains("claude-compaction-restore"));
    assert!(!prompt.contains("plugins"));
}

#[test]
fn restore_prompt_refuses_an_empty_or_oversized_marker() {
    let empty = build_post_compact_restore_prompt(&input("")).expect_err("empty");
    assert!(
        empty.to_string().contains("restore marker is empty"),
        "{empty}"
    );
    let big = "x".repeat(RESTORE_INLINE_LIMIT.saturating_add(1));
    let over = build_post_compact_restore_prompt(&input(&big)).expect_err("over");
    assert!(over.to_string().contains("inline limit"), "{over}");
    assert!(!over.to_string().contains("restore-pending"));
}

fn input(marker_json: &str) -> RestorePrompt<'_> {
    RestorePrompt {
        session_name: "claude",
        rig_home: Path::new("/home/rig"),
        marker_json,
        transcript_path: Some(Path::new("/tmp/t.jsonl")),
        session_id: Some("sid"),
        post_compact_instruction: None,
        post_compact_instruction_file: None,
        ignored_wrong_seat_extra: false,
    }
}
