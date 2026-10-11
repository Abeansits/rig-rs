//! ADR-0005 verdict table and the quiet-window property.

use std::time::{Duration, Instant, SystemTime};

use crewmux_mux::{
    Capture, ComposerState, DeferReason, GuardConfig, GuardInputs, GuardVerdict, evaluate,
};
use proptest::prelude::*;

fn inputs(in_mode: bool, tap: Option<Instant>, tmux: Option<SystemTime>) -> GuardInputs {
    GuardInputs {
        human_clients: Vec::new(),
        last_human_keystroke_tmux: tmux,
        last_human_keystroke_tap: tap,
        pane_in_mode: in_mode,
        tail: Capture::new(String::new(), (0, 0)),
        last_own_write: None,
    }
}

fn cfg(wall: SystemTime) -> GuardConfig {
    GuardConfig {
        quiet: Duration::from_secs(3),
        max_defer: Duration::from_secs(30),
        wall,
    }
}

fn reason(verdict: &GuardVerdict) -> Option<&DeferReason> {
    match verdict {
        GuardVerdict::Defer { reason, .. } | GuardVerdict::Escalate(reason) => Some(reason),
        GuardVerdict::Deliver => None,
    }
}

#[test]
fn rules_follow_the_table() {
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
    let recent_tap = now.checked_sub(Duration::from_secs(1)).expect("tap");
    let old_tap = now.checked_sub(Duration::from_secs(10)).expect("old");
    let recent_tmux = wall.checked_sub(Duration::from_secs(1)).expect("tmux");
    let cases = [
        (
            ComposerState::NotAtPrompt,
            inputs(false, None, None),
            false,
            DeferReason::NotAtPrompt,
            false,
        ),
        (
            ComposerState::AtPrompt,
            inputs(true, None, None),
            false,
            DeferReason::InCopyMode,
            false,
        ),
        (
            ComposerState::AtPrompt,
            inputs(false, Some(recent_tap), None),
            false,
            DeferReason::HumanTyping,
            false,
        ),
        (
            ComposerState::AtPrompt,
            inputs(false, None, Some(recent_tmux)),
            false,
            DeferReason::HumanTyping,
            false,
        ),
        (
            ComposerState::Draft {
                age: Duration::from_secs(1),
            },
            inputs(false, Some(old_tap), None),
            false,
            DeferReason::HumanDraft,
            false,
        ),
    ];
    let config = cfg(wall);
    for (composer, input, degraded, expect, escalate) in cases {
        let evaluated = evaluate(&input, composer, &config, now);
        assert_eq!(evaluated.guard_degraded, degraded);
        assert_eq!(reason(&evaluated.verdict), Some(&expect));
        assert_eq!(
            matches!(evaluated.verdict, GuardVerdict::Escalate(_)),
            escalate
        );
    }
    aged_draft_escalates(&config, now);
    unknown_is_degraded(&config, now);
    ready_delivers(now, old_tap, &config);
}

fn aged_draft_escalates(config: &GuardConfig, now: Instant) {
    let draft = evaluate(
        &inputs(false, None, None),
        ComposerState::Draft {
            age: Duration::from_secs(30),
        },
        config,
        now,
    );
    assert!(matches!(
        draft.verdict,
        GuardVerdict::Escalate(DeferReason::HumanDraft)
    ));
}

fn unknown_is_degraded(config: &GuardConfig, now: Instant) {
    let unknown = evaluate(
        &inputs(false, None, None),
        ComposerState::Unknown,
        config,
        now,
    );
    assert!(unknown.guard_degraded);
    assert!(matches!(unknown.verdict, GuardVerdict::Deliver));
    let unknown_mode = evaluate(
        &inputs(true, None, None),
        ComposerState::Unknown,
        config,
        now,
    );
    assert!(unknown_mode.guard_degraded);
    assert_eq!(
        reason(&unknown_mode.verdict),
        Some(&DeferReason::InCopyMode)
    );
}

fn ready_delivers(now: Instant, old_tap: Instant, config: &GuardConfig) {
    let ready = evaluate(
        &inputs(false, Some(old_tap), None),
        ComposerState::AtPrompt,
        config,
        now,
    );
    assert!(matches!(ready.verdict, GuardVerdict::Deliver));
    assert!(!ready.guard_degraded);
}

#[test]
fn exactly_three_seconds_delivers() {
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
    let tap = now.checked_sub(Duration::from_secs(3)).expect("tap");
    let tmux = wall.checked_sub(Duration::from_secs(3)).expect("tmux");
    let by_tap = evaluate(
        &inputs(false, Some(tap), None),
        ComposerState::AtPrompt,
        &cfg(wall),
        now,
    );
    let by_tmux = evaluate(
        &inputs(false, None, Some(tmux)),
        ComposerState::AtPrompt,
        &cfg(wall),
        now,
    );
    assert!(matches!(by_tap.verdict, GuardVerdict::Deliver));
    assert!(matches!(by_tmux.verdict, GuardVerdict::Deliver));
}

#[test]
fn future_keystroke_defers() {
    let now = Instant::now();
    let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
    let tap = now.checked_add(Duration::from_secs(5)).expect("tap");
    let tmux = wall.checked_add(Duration::from_secs(5)).expect("tmux");
    let by_tap = evaluate(
        &inputs(false, Some(tap), None),
        ComposerState::AtPrompt,
        &cfg(wall),
        now,
    );
    let by_tmux = evaluate(
        &inputs(false, None, Some(tmux)),
        ComposerState::AtPrompt,
        &cfg(wall),
        now,
    );
    assert_eq!(reason(&by_tap.verdict), Some(&DeferReason::HumanTyping));
    assert_eq!(reason(&by_tmux.verdict), Some(&DeferReason::HumanTyping));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]
    #[test]
    fn quiet_keystroke_never_delivers(
        tap_ms in 0u64..3_000,
        tmux_ms in 0u64..3_000,
        use_tap in any::<bool>(),
        use_tmux in any::<bool>(),
        composer in 0u8..4,
        in_mode in any::<bool>(),
    ) {
        prop_assume!(use_tap || use_tmux);
        let now = Instant::now();
        let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let tap = use_tap.then(|| now.checked_sub(Duration::from_millis(tap_ms)).expect("tap"));
        let tmux = use_tmux.then(|| wall.checked_sub(Duration::from_millis(tmux_ms)).expect("tmux"));
        let state = match composer {
            0 => ComposerState::AtPrompt,
            1 => ComposerState::NotAtPrompt,
            2 => ComposerState::Draft { age: Duration::from_secs(1) },
            _ => ComposerState::Unknown,
        };
        let evaluated = evaluate(&inputs(in_mode, tap, tmux), state, &cfg(wall), now);
        prop_assert!(!matches!(evaluated.verdict, GuardVerdict::Deliver));
    }
}
