//! Restore prompt golden. Plugin skills this build does not ship are absent.

use std::path::Path;

use crewmux_health::compaction::{RestorePrompt, build_post_compact_restore_prompt};

const GOLDEN: &str = "Please respond to this normal user message now by restoring this Claude session after compaction. This is the operator-authorized OpenRig restore request referenced by the compact summary; it is not local-command stdout or hook output. Restoration is the current task. Do not wait for a future user request or task assignment before reading the required files. If you did not compact (your earlier context is still present), say so and skip the restore reading. First, look for the pending restore marker at /marks/a.json. If the marker is missing and you have no restore map, rebuild a packet from this Claude JSONL transcript: /tmp/t.jsonl. Follow the marker's restoreInstruction and postCompactInstruction when present, read your newest restore map and use the restore packet as a lookup, then reply with: restored from packet at <path>; resumed at step <X>.";

#[test]
fn restore_prompt_golden() {
    let prompt = build_post_compact_restore_prompt(&RestorePrompt {
        session_name: "claude",
        rig_home: Path::new("/home/rig"),
        marker_path: Some(Path::new("/marks/a.json")),
        transcript_path: Some(Path::new("/tmp/t.jsonl")),
        session_id: Some("sid"),
        post_compact_instruction: None,
        post_compact_instruction_file: None,
        ignored_wrong_seat_extra: false,
    });
    assert_eq!(prompt, GOLDEN);
    assert!(!prompt.contains("SKILL.md"));
    assert!(!prompt.contains("refocusing"));
    assert!(!prompt.contains("claude-compaction-restore"));
    assert!(!prompt.contains("plugins"));
}
