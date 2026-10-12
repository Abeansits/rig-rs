//! A marker is acted on only after both completion signals confirm it.

use std::path::PathBuf;

use crewmux_health::compaction::{
    AttemptId, Book, CompactTrigger, HookEvent, RestoreMarker, is_actionable,
};

#[test]
fn unconfirmed_marker_ignored() {
    let marker = RestoreMarker {
        seat_key: "seat".to_owned(),
        attempt_id: "manual-1".to_owned(),
        trigger: CompactTrigger::Manual,
        managed: true,
        confirmed: false,
        session_id: "abc".to_owned(),
        transcript_path: PathBuf::new(),
        cwd: PathBuf::new(),
    };
    assert!(!is_actionable(&marker));
    let mut book = Book::new();
    book.request("seat", AttemptId("manual-1".into()), 0);
    let effect = book.on_hook(
        "seat",
        &HookEvent::PreCompact {
            session_id: "abc".to_owned(),
            trigger: CompactTrigger::Manual,
            custom_instructions: None,
            transcript_path: PathBuf::new(),
            cwd: PathBuf::new(),
        },
        1_000,
    );
    let written = effect.marker.expect("marker");
    assert!(!written.confirmed);
    assert!(!is_actionable(&written));
    assert!(book.due_restore("seat", true, 1_000 + 86_400_000).is_none());
}
