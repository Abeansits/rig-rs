//! An unsolicited automatic `PreCompact` leaves the managed marker bytes alone.

use std::path::PathBuf;

use crewmux_health::compaction::{
    AbortReason, AttemptId, AttemptState, Book, CompactTrigger, HookEvent, marker_bytes,
    marker_path,
};

#[test]
fn stray_auto_precompact_does_not_touch_managed_marker() {
    let root = std::env::temp_dir().join(format!("rig-stray-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&root));
    let mut book = Book::new();
    let managed_id = AttemptId("manual-seat-1".into());
    book.request("seat", managed_id, 0);
    let managed = book.on_hook("seat", &pre(CompactTrigger::Manual, "keep"), 1_000);
    let marker = managed.marker.expect("managed");
    let path = marker_path(&root, "seat", marker.attempt_id.as_str());
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    let bytes = marker_bytes(&marker).expect("bytes");
    std::fs::write(&path, &bytes).expect("write");
    let auto = book.on_hook("seat", &pre(CompactTrigger::Auto, ""), 2_000);
    let auto_marker = auto.marker.expect("auto");
    assert_ne!(auto_marker.attempt_id, marker.attempt_id);
    assert!(!auto_marker.managed);
    assert_eq!(std::fs::read(&path).expect("reread"), bytes);
    book.tick(2_000 + 120_000);
    assert_eq!(std::fs::read(&path).expect("after tick"), bytes);
    let attempts = book.for_seat("seat");
    assert!(attempts.iter().all(|attempt| {
        matches!(
            attempt.state,
            AttemptState::Aborted(AbortReason::NoPostCompact)
        )
    }));
    assert!(attempts.iter().any(|attempt| attempt.managed));
    assert!(attempts.iter().any(|attempt| !attempt.managed));
    assert!(
        book.due_restore("seat", true, 2_000 + 120_000 + 10_000)
            .is_none()
    );
    drop(std::fs::remove_dir_all(&root));
}

fn pre(trigger: CompactTrigger, instructions: &str) -> HookEvent {
    HookEvent::PreCompact {
        session_id: "abc123".to_owned(),
        trigger,
        custom_instructions: if instructions.is_empty() {
            None
        } else {
            Some(instructions.to_owned())
        },
        transcript_path: PathBuf::from("/tmp/t.jsonl"),
        cwd: PathBuf::from("/work"),
    }
}
