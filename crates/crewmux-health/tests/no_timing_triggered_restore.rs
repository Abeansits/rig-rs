//! Elapsed time aborts an unfinished attempt. It never restores.

use std::path::PathBuf;

use crewmux_health::compaction::{
    AbortReason, AttemptId, AttemptState, Book, CompactTrigger, Enforcer, EnforcerInput,
    EnforcerOutcome, HookEvent, Threshold,
};
use crewmux_types::{RuntimeKind, SeatId};

#[test]
fn no_timing_triggered_restore() {
    let mut book = Book::new();
    book.request("seat", AttemptId("manual-1".into()), 0);
    book.on_hook(
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
    let later = 1_000 + 120_000 + 1_500_000 + 600_000 + 10_000;
    assert!(book.tick(later));
    let attempt = book.for_seat("seat").into_iter().next().expect("attempt");
    assert!(matches!(
        attempt.state,
        AttemptState::Aborted(AbortReason::NoPostCompact)
    ));
    assert!(book.due_restore("seat", true, later).is_none());
    let mut enforcer = Enforcer::new();
    let prep = crewmux_health::compaction::PreparationState::idle();
    let outcome = enforcer.decide(&input(), &prep, later);
    assert!(!matches!(outcome, EnforcerOutcome::SendRestore(_)));
}

fn input() -> EnforcerInput {
    EnforcerInput {
        seat: SeatId::new(),
        session: "seat".to_owned(),
        runtime: RuntimeKind::ClaudeCode,
        used_percent: Some(90),
        sample_at_ms: Some(0),
        threshold: Threshold::some(70),
        enabled: true,
        ready: true,
        cwd: None,
        transcript_path: None,
        session_id: None,
        compact_instruction: String::new(),
        pre_compact_instruction: None,
        rig_home: PathBuf::from("/home"),
        attempt_id: "a".to_owned(),
        skip_map: false,
    }
}
