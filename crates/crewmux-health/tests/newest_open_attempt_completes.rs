//! A stray auto `PreCompact` and a later auto compaction share one restore.

use std::path::PathBuf;

use crewmux_health::compaction::{AttemptState, Book, CompactTrigger, HookEvent, SessionSource};

#[test]
fn newest_open_attempt_completes() {
    let mut book = Book::new();
    book.on_hook("seat", &pre(CompactTrigger::Auto), 1_000);
    book.on_hook("seat", &pre(CompactTrigger::Auto), 2_000);
    book.on_hook("seat", &start(), 3_000);
    book.on_hook("seat", &post(), 4_000);
    let attempts = book.for_seat("seat");
    assert_eq!(attempts.len(), 2);
    let older = attempts.first().expect("older");
    let newer = attempts.get(1).expect("newer");
    assert!(matches!(
        older.state,
        AttemptState::Failed(ref reason) if reason == "superseded"
    ));
    assert!(matches!(newer.state, AttemptState::Completed { .. }));
    let due = book
        .due_restore("seat", true, 4_000 + 10_000)
        .expect("one restore");
    assert_eq!(due.id.as_str(), newer.id.as_str());
}

fn pre(trigger: CompactTrigger) -> HookEvent {
    HookEvent::PreCompact {
        session_id: "sess".to_owned(),
        trigger,
        custom_instructions: None,
        transcript_path: PathBuf::from("/tmp/t.jsonl"),
        cwd: PathBuf::from("/work"),
    }
}

fn start() -> HookEvent {
    HookEvent::SessionStart {
        session_id: "sess".to_owned(),
        source: SessionSource::Compact,
        context_tokens: None,
    }
}

fn post() -> HookEvent {
    HookEvent::PostCompact {
        session_id: "sess".to_owned(),
        trigger: CompactTrigger::Auto,
        compact_summary: "sum".to_owned(),
    }
}
