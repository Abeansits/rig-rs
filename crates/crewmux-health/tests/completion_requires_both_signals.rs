//! Completion needs `SessionStart` compact and `PostCompact`, either order.
#![expect(
    clippy::expect_used,
    reason = "missing attempt state should fail the test"
)]

use std::path::PathBuf;

use crewmux_health::compaction::{
    AttemptId, AttemptState, Book, CompactTrigger, HookEvent, SessionSource,
};

#[test]
fn completion_requires_both_signals() {
    assert_completed(true);
    assert_completed(false);
    let mut book = Book::new();
    book.request("seat", AttemptId("manual-1".into()), 0);
    book.on_hook("seat", &pre("s1", CompactTrigger::Manual), 1_000);
    let mismatched = book.on_hook("seat", &start("other"), 2_000);
    assert!(mismatched.ignored_session_mismatch);
    book.on_hook("seat", &post("s1", CompactTrigger::Manual, "sum"), 3_000);
    assert!(book.due_restore("seat", true, 3_000 + 10_000).is_none());
}

fn assert_completed(start_first: bool) {
    let mut book = Book::new();
    book.request("seat", AttemptId("manual-1".into()), 0);
    book.on_hook("seat", &pre("s1", CompactTrigger::Manual), 1_000);
    let (first, second) = if start_first {
        (start("s1"), post("s1", CompactTrigger::Manual, "sum"))
    } else {
        (post("s1", CompactTrigger::Manual, "sum"), start("s1"))
    };
    book.on_hook("seat", &first, 2_000);
    assert!(book.due_restore("seat", true, 9_000_000).is_none());
    let effect = book.on_hook("seat", &second, 3_000);
    let marker = effect.marker.expect("confirmed marker");
    assert!(marker.confirmed);
    let attempt = book.for_seat("seat").into_iter().next().expect("attempt");
    assert!(matches!(attempt.state, AttemptState::Completed { .. }));
    assert!(attempt.summary_sha_hex().is_some());
    assert_eq!(attempt.summary_len, Some(3));
    assert!(book.due_restore("seat", true, 3_000).is_none());
    assert!(book.due_restore("seat", false, 13_000).is_none());
    assert!(book.due_restore("seat", true, 13_000).is_some());
    let doc = serde_json::to_string(attempt).expect("doc");
    let loaded: crewmux_health::compaction::Attempt = serde_json::from_str(&doc).expect("load");
    assert_eq!(loaded.summary_len, Some(3));
}

#[test]
fn cancel_drops_a_due_restore() {
    let mut book = Book::new();
    book.request("seat", AttemptId("manual-1".into()), 0);
    book.on_hook("seat", &pre("s1", CompactTrigger::Manual), 1_000);
    book.on_hook("seat", &start("s1"), 2_000);
    book.on_hook("seat", &post("s1", CompactTrigger::Manual, "sum"), 3_000);
    assert!(book.due_restore("seat", true, 13_000).is_some());
    assert!(book.cancel("seat"));
    assert!(book.due_restore("seat", true, 13_000).is_none());
    let attempt = book.for_seat("seat").into_iter().next().expect("attempt");
    assert!(matches!(attempt.state, AttemptState::Cancelled));
}

fn pre(session: &str, trigger: CompactTrigger) -> HookEvent {
    HookEvent::PreCompact {
        session_id: session.to_owned(),
        trigger,
        custom_instructions: Some("keep".to_owned()),
        transcript_path: PathBuf::from("/tmp/t.jsonl"),
        cwd: PathBuf::from("/work"),
    }
}

fn start(session: &str) -> HookEvent {
    HookEvent::SessionStart {
        session_id: session.to_owned(),
        source: SessionSource::Compact,
        context_tokens: None,
    }
}

fn post(session: &str, trigger: CompactTrigger, summary: &str) -> HookEvent {
    HookEvent::PostCompact {
        session_id: session.to_owned(),
        trigger,
        compact_summary: summary.to_owned(),
    }
}
