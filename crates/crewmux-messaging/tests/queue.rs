#![expect(
    clippy::expect_used,
    reason = "integration-test helpers abort when a fixture cannot be built"
)]
#![expect(
    clippy::panic,
    reason = "helpers outside #[test] abort when a ladder action has the wrong shape"
)]

//! Queue state machine, claims, closure, crash hook, retention, and the wake ladder.

use std::time::Duration;

use crewmux_core::Timestamp;
use crewmux_messaging::{
    Actor, Error, InboxEntry, InboxState, LadderView, Mailboxes, NewItem, NudgeRecord, QueueBook,
    QueueState, Rung, SessionRef, Update, WakeAction, WakeConfig, WakeHumanRung, WakeLadder,
    escalation_reason, leaks_body, nudge_body, transition_note,
};
use crewmux_types::QItemId;

fn session(text: &str) -> SessionRef {
    SessionRef::new(text).expect("session")
}

fn at(millis: i64) -> Timestamp {
    Timestamp::from_millis(millis)
}

fn item(book: &mut QueueBook, body: &str) -> crewmux_messaging::QueueItem {
    book.create(
        NewItem {
            id: None,
            source: session("alice@rig"),
            destination: session("bob@rig"),
            body: body.to_owned(),
            priority: 0,
            tier: Some("routine".to_owned()),
            tags: Vec::new(),
            handed_off_from: None,
            last_nudge: None,
            explicit_human: false,
            fingerprint: String::new(),
        },
        at(1_000),
    )
    .expect("create")
}

fn update_state(
    book: &mut QueueBook,
    id: &QItemId,
    state: QueueState,
    reason: Option<&str>,
    target: Option<&str>,
    reopen: bool,
) -> Result<crewmux_messaging::Transition, Error> {
    book.update(
        id,
        Update {
            state: Some(state),
            actor: Actor::Session(session("bob@rig")),
            note: "note".to_owned(),
            closure_reason: reason.map(str::to_owned),
            closure_target: target.map(str::to_owned),
            reopen,
            blocked_on: None,
        },
        at(2_000),
    )
}

#[test]
fn every_state_pair_matches_the_terminal_and_closure_rules() {
    let id = QItemId::new("qitem-1").expect("id");
    for from in QueueState::all() {
        for to in QueueState::all() {
            let leaving = from != to;
            let reopen = from.is_guarded() && leaving;
            let err = crewmux_messaging::admit(&id, from, &pair_update(to, reopen, leaving));
            let expect = if from.is_guarded() && leaving {
                to.is_active()
            } else {
                true
            };
            assert_eq!(err.is_ok(), expect, "{from:?} -> {to:?} {err:?}");
        }
    }
}

fn pair_update(to: QueueState, reopen: bool, with_closure: bool) -> Update {
    let (reason, target) = if with_closure {
        match to {
            QueueState::Done => (Some("no-follow-on".to_owned()), None),
            QueueState::HandedOff => (
                Some("handed_off_to".to_owned()),
                Some("carol@rig".to_owned()),
            ),
            QueueState::Pending
            | QueueState::InProgress
            | QueueState::Blocked
            | QueueState::Failed
            | QueueState::Denied
            | QueueState::Canceled => (None, None),
        }
    } else {
        (None, None)
    };
    Update {
        state: Some(to),
        actor: Actor::System,
        note: "note".to_owned(),
        closure_reason: reason,
        closure_target: target,
        reopen,
        blocked_on: None,
    }
}

#[test]
fn done_without_closure_uses_the_typescript_sentence() {
    let mut book = QueueBook::new();
    let created = item(&mut book, "x");
    book.claim(&created.id, &session("bob@rig"), at(2_000), None)
        .expect("claim");
    let err = update_state(&mut book, &created.id, QueueState::Done, None, None, false)
        .expect_err("closure");
    match err {
        Error::Rejected { code, message } => {
            assert_eq!(code, "missing_closure_reason");
            assert!(
                message.contains("state=done requires closure_reason"),
                "{message}"
            );
            assert!(message.contains("handed_off_to"), "{message}");
            assert!(message.contains("superseded"), "{message}");
        }
        Error::NotFound(_) | Error::AlreadyClaimed { .. } | Error::Crashed(_) => {
            panic!("unexpected {err:?}")
        }
    }
}

#[test]
fn each_closure_reason_is_accepted() {
    for reason in crewmux_messaging::CLOSURE_REASONS {
        let mut book = QueueBook::new();
        let created = item(&mut book, reason);
        book.claim(&created.id, &session("bob@rig"), at(2_000), None)
            .expect("claim");
        let needs_target = matches!(
            *reason,
            "handed_off_to" | "blocked_on" | "escalation" | "superseded"
        );
        let target = needs_target.then_some("downstream-target");
        update_state(
            &mut book,
            &created.id,
            QueueState::Done,
            Some(reason),
            target,
            false,
        )
        .unwrap_or_else(|err| panic!("{reason}: {err}"));
        let stored = book.get(&created.id).expect("row");
        assert_eq!(stored.state, QueueState::Done);
        let closure = stored.closure.as_ref().expect("closure");
        assert_eq!(closure.reason, *reason);
    }
}

#[test]
fn handoff_is_one_swap_and_a_crash_keeps_the_before_state() {
    let mut book = QueueBook::new();
    let created = item(&mut book, "do it");
    for point in ["after-source", "after-successor", "before-commit"] {
        let mut crashed = book.clone();
        crashed.arm_crash(point);
        let err = crashed
            .handoff(
                &created.id,
                &session("carol@rig"),
                &session("bob@rig"),
                at(3_000),
                false,
            )
            .expect_err(point);
        assert_eq!(err, Error::Crashed(point));
        assert_eq!(
            crashed.get(&created.id).expect("src").state,
            QueueState::Pending
        );
        assert_eq!(crashed.items().len(), 1, "{point}");
    }
    let (closed, created_next) = book
        .handoff(
            &created.id,
            &session("carol@rig"),
            &session("bob@rig"),
            at(3_000),
            false,
        )
        .expect("handoff");
    assert_eq!(closed.state, QueueState::HandedOff);
    assert_eq!(
        closed.closure.as_ref().map(|c| c.reason.as_str()),
        Some("handed_off_to")
    );
    assert_eq!(
        closed.handed_off_to.as_ref().map(SessionRef::as_str),
        Some("carol@rig")
    );
    assert_eq!(created_next.state, QueueState::Pending);
    assert_eq!(created_next.handed_off_from.as_ref(), Some(&created.id));
    assert_eq!(created_next.destination.as_str(), "carol@rig");
}

#[test]
fn handoff_of_a_terminal_row_is_refused() {
    let mut book = QueueBook::new();
    let created = item(&mut book, "x");
    book.claim(&created.id, &session("bob@rig"), at(2_000), None)
        .expect("claim");
    update_state(
        &mut book,
        &created.id,
        QueueState::Done,
        Some("no-follow-on"),
        None,
        false,
    )
    .expect("done");
    let err = book
        .handoff(
            &created.id,
            &session("carol@rig"),
            &session("bob@rig"),
            at(4_000),
            false,
        )
        .expect_err("terminal");
    let Error::Rejected { message, .. } = err else {
        panic!("expected rejection");
    };
    assert!(message.contains("terminal"), "{message}");
}

#[test]
fn claim_is_exclusive_and_computes_the_tier_deadline() {
    let mut book = QueueBook::new();
    let created = item(&mut book, "work");
    let claimed = book
        .claim(&created.id, &session("bob@rig"), at(5_000), None)
        .expect("claim");
    assert_eq!(claimed.state, QueueState::InProgress);
    let due = claimed.closure_required_at.expect("sla");
    assert_eq!(due.millis() - 5_000, 4 * 60 * 60 * 1_000);
    let err = book
        .claim(&created.id, &session("bob@rig"), at(6_000), None)
        .expect_err("second");
    assert!(matches!(err, Error::AlreadyClaimed { .. }));
    let err = book
        .claim(&created.id, &session("carol@rig"), at(6_000), Some("  "))
        .expect_err("blank force");
    assert!(matches!(err, Error::AlreadyClaimed { .. }));
    book.claim(
        &created.id,
        &session("carol@rig"),
        at(7_000),
        Some("taking over"),
    )
    .expect("force");
    assert_eq!(
        book.get(&created.id)
            .and_then(|row| row.claimed_by.as_ref())
            .map(SessionRef::as_str),
        Some("carol@rig")
    );
    book.unclaim(&created.id, &session("carol@rig"), "pause", at(8_000))
        .expect("unclaim");
    let released = book.get(&created.id).expect("row");
    assert_eq!(released.state, QueueState::Pending);
    assert!(released.claimed_by.is_none());
    let err = book
        .claim(&created.id, &session("dave@rig"), at(9_000), None)
        .expect_err("mismatch");
    let Error::Rejected { code, message } = err else {
        panic!("mismatch type");
    };
    assert_eq!(code, "claim_destination_mismatch");
    assert!(
        message.contains("destination is bob@rig, not dave@rig"),
        "{message}"
    );
}

#[test]
fn retention_keeps_non_terminal_rows() {
    let mut book = QueueBook::new();
    let done = item(&mut book, "old");
    book.claim(&done.id, &session("bob@rig"), at(2_000), None)
        .expect("claim");
    update_state(
        &mut book,
        &done.id,
        QueueState::Done,
        Some("no-follow-on"),
        None,
        false,
    )
    .expect("done");
    let live = item(&mut book, "live");
    let canceled = item(&mut book, "canceled");
    update_state(
        &mut book,
        &canceled.id,
        QueueState::Canceled,
        None,
        None,
        false,
    )
    .expect("cancel");
    let cutoff = at(1_000_000_000);
    let ids = book.retention_ids(cutoff);
    assert!(ids.contains(&done.id));
    assert!(!ids.contains(&live.id));
    assert!(!ids.contains(&canceled.id));
    assert!(QueueState::Done.is_terminal());
    assert!(QueueState::HandedOff.is_terminal());
    assert!(!QueueState::Canceled.is_terminal());
}

#[test]
fn ladder_retries_then_escalates_and_survives_a_restart() {
    let cfg = WakeConfig {
        retry_cap: 3,
        retry_interval: Duration::from_secs(10),
        unconfirmed_window: Duration::from_mins(30),
        human_rung: WakeHumanRung::ExplicitOnly,
    };
    let ladder = WakeLadder::new(cfg.clone());
    let mut book = QueueBook::new();
    let baton = book
        .create(
            baton_item("secret-body-token", "failed:tmux session not found", false),
            at(0),
        )
        .expect("baton");
    let now = record_failed_retries(&ladder, &cfg, &mut book, &baton.id);
    let view = LadderView::from_item(
        book.get(&baton.id).expect("row"),
        &book.transitions(&baton.id),
    );
    let actions = ladder.tick(&view, now);
    match actions.first() {
        Some(WakeAction::Escalate {
            rung: Rung::DestinationOrchestrator,
            aggregate_key,
        }) => {
            assert_eq!(aggregate_key, "worker@r");
        }
        other => panic!("orchestrator rung, got {other:?}"),
    }
    let reason = escalation_reason(&view, &cfg, now);
    assert!(reason.contains("wake failed 3 times"), "{reason}");
    let note = transition_note(actions.first().expect("action"), 3, 3, &reason);
    assert!(note.starts_with("escalation-rung:"), "{note}");
    assert!(note.contains("orchestrator"), "{note}");
    book.append_note(&baton.id, &note, None, None, now)
        .expect("rung");
    let restarted = book.clone();
    let view = LadderView::from_item(
        restarted.get(&baton.id).expect("row"),
        &restarted.transitions(&baton.id),
    );
    let next = ladder.tick(&view, now.saturating_add(Duration::from_secs(11)));
    assert!(matches!(
        next.first(),
        Some(WakeAction::Escalate {
            rung: Rung::Operator,
            ..
        })
    ));
}

fn record_failed_retries(
    ladder: &WakeLadder,
    cfg: &WakeConfig,
    book: &mut QueueBook,
    id: &QItemId,
) -> Timestamp {
    let mut now = at(20_000);
    for expected in 1..=3 {
        let view = LadderView::from_item(book.get(id).expect("row"), &book.transitions(id));
        let actions = ladder.tick(&view, now);
        let WakeAction::Nudge { qitem, target } = actions.first().cloned().expect("nudge") else {
            panic!("expected nudge");
        };
        assert_eq!(&qitem, id);
        assert_eq!(target.as_str(), "worker@r");
        let text = nudge_body(&qitem, &view.body);
        assert!(!leaks_body(&text, "secret-body-token"), "{text}");
        let note = transition_note(
            actions.first().expect("action"),
            expected,
            cfg.retry_cap,
            "",
        );
        assert!(note.starts_with("wake-attempt:"), "{note}");
        assert!(note.contains(&format!("{expected}/3")), "{note}");
        book.append_note(id, &note, Some(&text), Some("failed:again"), now)
            .expect("note");
        if expected < 3 {
            let again = ladder.tick(
                &LadderView::from_item(book.get(id).expect("row"), &book.transitions(id)),
                now,
            );
            assert_eq!(again.len(), 0, "interval");
        }
        now = now.saturating_add(Duration::from_secs(11));
    }
    now
}

#[test]
fn unconfirmed_is_not_renudged_inside_the_window() {
    let ladder = WakeLadder::new(WakeConfig {
        retry_interval: Duration::from_secs(1),
        unconfirmed_window: Duration::from_mins(30),
        ..WakeConfig::default()
    });
    let mut book = QueueBook::new();
    let baton = book
        .create(baton_item("body", "delivered-ack-pending", false), at(0))
        .expect("baton");
    let view = LadderView::from_item(
        book.get(&baton.id).expect("row"),
        &book.transitions(&baton.id),
    );
    assert_eq!(ladder.tick(&view, at(60_000)).len(), 0);
    let later = ladder.tick(&view, at(31 * 60 * 1_000));
    assert!(matches!(
        later.first(),
        Some(WakeAction::Escalate {
            rung: Rung::DestinationOrchestrator,
            ..
        })
    ));
    let reason = escalation_reason(&view, &WakeConfig::default(), at(31 * 60 * 1_000));
    assert!(reason.contains("unconfirmed"), "{reason}");
}

#[test]
fn explicit_only_skips_a_human_seat_unless_addressed() {
    let always = WakeLadder::new(WakeConfig {
        human_rung: WakeHumanRung::Always,
        retry_interval: Duration::from_secs(1),
        ..WakeConfig::default()
    });
    let explicit = WakeLadder::new(WakeConfig {
        human_rung: WakeHumanRung::ExplicitOnly,
        retry_interval: Duration::from_secs(1),
        ..WakeConfig::default()
    });
    let mut book = QueueBook::new();
    let quiet = book.create(human_baton(false), at(0)).expect("quiet");
    let named = book.create(human_baton(true), at(0)).expect("named");
    let view = |book: &QueueBook, id: &QItemId| {
        LadderView::from_item(book.get(id).expect("row"), &book.transitions(id))
    };
    assert_eq!(explicit.tick(&view(&book, &quiet.id), at(10_000)).len(), 0);
    assert!(
        explicit
            .tick(&view(&book, &named.id), at(10_000))
            .iter()
            .any(|action| matches!(action, WakeAction::Nudge { .. }))
    );
    assert!(
        always
            .tick(&view(&book, &quiet.id), at(10_000))
            .iter()
            .any(|action| matches!(action, WakeAction::Nudge { .. }))
    );
}

#[test]
fn five_batons_share_one_orchestrator_escalation() {
    let cfg = WakeConfig {
        retry_cap: 1,
        retry_interval: Duration::from_secs(1),
        ..WakeConfig::default()
    };
    let ladder = WakeLadder::new(cfg);
    let mut actions = Vec::new();
    for _ in 0..5 {
        let mut book = QueueBook::new();
        let baton = book
            .create(baton_item("x", "failed:down", false), at(0))
            .expect("baton");
        book.append_note(
            &baton.id,
            "wake-attempt: 1/1 outcome=failed",
            Some("pointer"),
            Some("failed:down"),
            at(1_000),
        )
        .expect("attempt");
        let view = LadderView::from_item(
            book.get(&baton.id).expect("row"),
            &book.transitions(&baton.id),
        );
        actions.extend(ladder.tick(&view, at(5_000)));
    }
    let deduped = crewmux_messaging::dedup_escalations(actions);
    assert_eq!(deduped.len(), 1);
}

#[test]
fn inbox_absorbs_and_denies() {
    let mut mail = Mailboxes::new();
    let id = mail
        .drop_inbox(InboxEntry {
            id: "inbox-1".to_owned(),
            destination: session("bob@rig"),
            sender: session("alice@rig"),
            body: "hello".to_owned(),
            state: InboxState::Pending,
            dropped_at: at(1),
            absorbed_qitem: None,
            denied_reason: None,
        })
        .expect("drop");
    let denied = mail.deny(&id.id, "no").expect("deny");
    assert_eq!(denied.state, InboxState::Denied);
    assert!(mail.absorb(&id.id, QItemId::generate()).is_err());
}

fn baton_item(body: &str, result: &str, human: bool) -> NewItem {
    let destination = if human {
        session("human@kernel")
    } else {
        session("worker@r")
    };
    NewItem {
        id: None,
        source: session("alice@rig"),
        destination,
        body: body.to_owned(),
        priority: 0,
        tier: None,
        tags: Vec::new(),
        handed_off_from: Some(QItemId::new("qitem-source").expect("id")),
        last_nudge: Some(NudgeRecord {
            at: at(0),
            result: result.to_owned(),
            text: "prior".to_owned(),
        }),
        explicit_human: human,
        fingerprint: String::new(),
    }
}

fn human_baton(explicit: bool) -> NewItem {
    let mut item = baton_item("please", "failed:guard", true);
    item.explicit_human = explicit;
    item.destination = session("human@kernel");
    item
}
