//! Prep, compact, and restore text is withheld until the seat is Ready.

use std::path::PathBuf;

use crewmux_health::compaction::{
    Enforcer, EnforcerInput, EnforcerOutcome, EnforcerSkipReason, Threshold, gated_submit,
};
use crewmux_types::{RuntimeKind, SeatId};

#[test]
fn compaction_never_submits_when_not_ready() {
    let mut enforcer = Enforcer::new();
    let prep = crewmux_health::compaction::PreparationState::idle();
    let outcome = enforcer.decide_manual(&input(false), &prep, 0);
    assert!(matches!(
        outcome,
        EnforcerOutcome::Skip(EnforcerSkipReason::SendFailed)
    ));
    assert!(gated_submit(false, &outcome).is_none());
    assert!(gated_submit(false, &EnforcerOutcome::SendCompact("/compact".into())).is_none());
    assert!(gated_submit(false, &EnforcerOutcome::SendRestore("restore".into())).is_none());
    let sent = EnforcerOutcome::SendPrep("prep".into());
    assert_eq!(gated_submit(true, &sent).as_deref(), Some("prep"));
}

fn input(ready: bool) -> EnforcerInput {
    EnforcerInput {
        seat: SeatId::new(),
        session: "claude".to_owned(),
        runtime: RuntimeKind::ClaudeCode,
        used_percent: Some(80),
        sample_at_ms: Some(0),
        threshold: Threshold::some(70),
        enabled: true,
        ready,
        cwd: None,
        transcript_path: None,
        session_id: None,
        compact_instruction: "keep".to_owned(),
        pre_compact_instruction: None,
        rig_home: PathBuf::from("/home"),
        attempt_id: "manual-1".to_owned(),
        skip_map: true,
    }
}
