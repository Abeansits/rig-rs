//! Keepalive decisions, deadline sub-states, and the sha256 message fixtures.
#![expect(
    clippy::expect_used,
    reason = "setup failures abort the integration test"
)]

use crewmux_core::Timestamp;
use crewmux_messaging::{Actor, NewItem, QueueBook, QueueState, SessionRef, Update, nudge_pointer};
use crewmux_orchestration::{
    Engine, Instance, KeepaliveJob, MAX_HOPS_BASELINE_V1, STUCK_THRESHOLD_SECONDS, Status,
    WatchAction, WatchFail, evaluate_deadline, evaluate_keepalive, exceeds_max_hops,
    keepalive_message, stuck_message,
};
use crewmux_types::{QItemId, WorkflowInstanceId};
use sha2::{Digest, Sha256};

const CREATED_MS: i64 = 1_777_791_600_000;

fn session(text: &str) -> SessionRef {
    SessionRef::new(text).expect("session")
}

fn qid(text: &str) -> QItemId {
    QItemId::new(text).expect("qitem")
}

fn instance(id: &str, status: Status, frontier: &[&str]) -> Instance {
    Instance {
        id: WorkflowInstanceId::new(id).expect("id"),
        workflow_name: "wf".to_owned(),
        workflow_version: "1".to_owned(),
        created_by: session("creator@rig"),
        created_at: Timestamp::from_millis(CREATED_MS),
        status,
        frontier: frontier.iter().copied().map(qid).collect(),
        current_step_id: None,
        hop_count: 0,
        fallback_synthesis: None,
        last_decision: None,
        completed_at: None,
        version: 1,
        resume_count: 0,
        hops_baseline: 0,
        bound_rig: None,
    }
}

fn engine(rows: Vec<Instance>) -> Engine {
    Engine {
        instances: rows,
        ..Engine::default()
    }
}

fn packet(book: &mut QueueBook, id: &str, dest: &str, at: i64, tier: Option<&str>) {
    book.create(
        NewItem {
            id: Some(qid(id)),
            source: session("src@r"),
            destination: session(dest),
            body: "x".to_owned(),
            priority: 0,
            tier: tier.map(str::to_owned),
            tags: Vec::new(),
            handed_off_from: None,
            last_nudge: None,
            explicit_human: false,
            fingerprint: "fp".to_owned(),
        },
        Timestamp::from_millis(at),
    )
    .expect("create");
}

fn job<'a>(
    instance_id: Option<&'a str>,
    observer: Option<&'a str>,
    gated: bool,
    last: Option<&'a str>,
) -> KeepaliveJob<'a> {
    KeepaliveJob {
        job_id: "job-1",
        target_session: "registered@rig",
        message: None,
        instance_id,
        packet_id: None,
        observer_session: observer,
        observer_sessions: &[],
        deadline_gated: gated,
        last_fired_condition: last,
    }
}

fn seeded() -> (Engine, QueueBook) {
    let mut book = QueueBook::new();
    packet(&mut book, "q-1", "owner1@rig", CREATED_MS, None);
    packet(&mut book, "q-2", "owner2@rig", CREATED_MS, None);
    packet(&mut book, "q-3", "waiter@rig", CREATED_MS, None);
    let rows = engine(vec![
        instance("inst-active", Status::Active, &["q-1", "q-2"]),
        instance("inst-completed", Status::Completed, &[]),
        instance("inst-waiting", Status::Waiting, &["q-3"]),
        instance("inst-empty", Status::Active, &[]),
    ]);
    (rows, book)
}

fn send(action: WatchAction) -> Option<(String, serde_json::Value, String)> {
    match action {
        WatchAction::Send {
            target,
            message,
            notes,
            ..
        } => Some((target, notes, message)),
        WatchAction::Skip { .. } | WatchAction::Terminal { .. } => None,
    }
}

#[test]
fn deadline_gated_healthy_is_a_quiet_skip() {
    let (engine, book) = seeded();
    let action = evaluate_keepalive(
        &engine,
        &book,
        &job(Some("inst-active"), None, true, None),
        Timestamp::from_millis(CREATED_MS),
    )
    .expect("eval");
    match action {
        WatchAction::Skip { reason } => assert_eq!(reason, "workflow_healthy_deadline_gated"),
        WatchAction::Send { .. } | WatchAction::Terminal { .. } => panic!("skip"),
    }
}

#[test]
fn active_instance_with_frontier_sends_to_the_first_owner() {
    let (engine, book) = seeded();
    let (target, notes, message) = send(
        evaluate_keepalive(
            &engine,
            &book,
            &job(Some("inst-active"), None, false, None),
            Timestamp::from_millis(CREATED_MS),
        )
        .expect("eval"),
    )
    .expect("send");
    assert_eq!(target, "owner1@rig");
    assert!(message.contains("Workflow keepalive"));
    assert!(message.contains("inst-active"));
    assert_eq!(notes["frontierLength"], 2);
}

#[test]
fn waiting_status_is_also_eligible() {
    let (engine, book) = seeded();
    let (target, _, _) = send(
        evaluate_keepalive(
            &engine,
            &book,
            &job(Some("inst-waiting"), None, false, None),
            Timestamp::from_millis(CREATED_MS),
        )
        .expect("eval"),
    )
    .expect("send");
    assert_eq!(target, "waiter@rig");
}

#[test]
fn completed_status_is_terminal_workflow_not_active() {
    let (engine, book) = seeded();
    let action = evaluate_keepalive(
        &engine,
        &book,
        &job(Some("inst-completed"), None, false, None),
        Timestamp::from_millis(CREATED_MS),
    )
    .expect("eval");
    match action {
        WatchAction::Terminal { reason, .. } => assert_eq!(reason, "workflow_not_active"),
        WatchAction::Send { .. } | WatchAction::Skip { .. } => panic!("terminal"),
    }
}

#[test]
fn empty_frontier_sends_to_the_creator() {
    let (engine, book) = seeded();
    let (target, _, _) = send(
        evaluate_keepalive(
            &engine,
            &book,
            &job(Some("inst-empty"), None, false, None),
            Timestamp::from_millis(CREATED_MS),
        )
        .expect("eval"),
    )
    .expect("send");
    assert_eq!(target, "creator@rig");
}

#[test]
fn missing_instance_is_terminal() {
    let (engine, book) = seeded();
    let action = evaluate_keepalive(
        &engine,
        &book,
        &job(Some("non-existent"), None, false, None),
        Timestamp::from_millis(CREATED_MS),
    )
    .expect("eval");
    match action {
        WatchAction::Terminal { reason, .. } => assert_eq!(reason, "workflow_instance_missing"),
        WatchAction::Send { .. } | WatchAction::Skip { .. } => panic!("terminal"),
    }
}

#[test]
fn missing_instance_id_is_policy_spec_invalid() {
    let (engine, book) = seeded();
    let err = evaluate_keepalive(
        &engine,
        &book,
        &job(None, None, false, None),
        Timestamp::from_millis(CREATED_MS),
    )
    .expect_err("missing");
    assert_eq!(err, WatchFail);
    assert_eq!(err.to_string(), "policy_spec_invalid");
}

#[test]
fn observer_session_is_an_additional_target() {
    let (engine, book) = seeded();
    let (_, notes, _) = send(
        evaluate_keepalive(
            &engine,
            &book,
            &job(Some("inst-active"), Some("observer@rig"), false, None),
            Timestamp::from_millis(CREATED_MS),
        )
        .expect("eval"),
    )
    .expect("send");
    let extra = notes["additionalRoutingTargets"]
        .as_array()
        .expect("targets");
    assert!(extra.iter().any(|value| value == "observer@rig"));
}

#[test]
fn policy_reads_the_workflow_document() {
    let (engine, book) = seeded();
    let action = evaluate_keepalive(
        &engine,
        &book,
        &job(Some("inst-active"), None, false, None),
        Timestamp::from_millis(CREATED_MS),
    )
    .expect("eval");
    assert!(matches!(
        action,
        WatchAction::Send { .. } | WatchAction::Terminal { .. } | WatchAction::Skip { .. }
    ));
}

#[test]
fn sub_state_1_closure_required_at() {
    let mut book = QueueBook::new();
    packet(&mut book, "q-1", "owner1@rig", CREATED_MS, Some("fast"));
    let id = qid("q-1");
    book.claim(
        &id,
        &session("owner1@rig"),
        Timestamp::from_millis(CREATED_MS),
        None,
    )
    .expect("claim");
    let row = instance("inst-active", Status::Active, &["q-1"]);
    let packet = book.get(&id).expect("packet").clone();
    let before = evaluate_deadline(
        &row,
        std::slice::from_ref(&packet),
        Timestamp::from_millis(CREATED_MS + 30 * 60 * 1000 - 1000),
    );
    assert_eq!(before.state, "healthy");
    let after = evaluate_deadline(
        &row,
        &[packet],
        Timestamp::from_millis(CREATED_MS + 30 * 60 * 1000),
    );
    assert_eq!(after.state, "overdue-claimed");
    assert_eq!(
        after.evidence.expect("evidence").anchor,
        "closure_required_at"
    );
}

#[test]
fn sub_state_2_claimed_at() {
    let mut book = QueueBook::new();
    packet(&mut book, "q-1", "owner1@rig", CREATED_MS, None);
    let id = qid("q-1");
    book.claim(
        &id,
        &session("owner1@rig"),
        Timestamp::from_millis(CREATED_MS),
        None,
    )
    .expect("claim");
    let row = instance("inst-active", Status::Active, &["q-1"]);
    let packet = book.get(&id).expect("packet").clone();
    assert!(packet.closure_required_at.is_none());
    let threshold = STUCK_THRESHOLD_SECONDS * 1000;
    let before = evaluate_deadline(
        &row,
        std::slice::from_ref(&packet),
        Timestamp::from_millis(CREATED_MS + threshold - 1000),
    );
    assert_eq!(before.state, "healthy");
    let after = evaluate_deadline(
        &row,
        &[packet],
        Timestamp::from_millis(CREATED_MS + threshold),
    );
    assert_eq!(after.state, "overdue-claimed");
    assert_eq!(after.evidence.expect("evidence").anchor, "claimed_at");
}

#[test]
fn blocked_frontier_packet_is_healthy() {
    let mut book = QueueBook::new();
    packet(&mut book, "q-1", "owner1@rig", CREATED_MS, None);
    let id = qid("q-1");
    book.update(
        &id,
        Update {
            state: Some(QueueState::Blocked),
            actor: Actor::Session(session("owner1@rig")),
            note: "park".to_owned(),
            closure_reason: Some("blocked_on".to_owned()),
            closure_target: Some("external@rig".to_owned()),
            reopen: false,
            blocked_on: Some(session("external@rig")),
        },
        Timestamp::from_millis(CREATED_MS),
    )
    .expect("block");
    let row = instance("inst-waiting", Status::Waiting, &["q-1"]);
    let packet = book.get(&id).expect("packet").clone();
    let verdict = evaluate_deadline(
        &row,
        &[packet],
        Timestamp::from_millis(CREATED_MS + 10 * STUCK_THRESHOLD_SECONDS * 1000),
    );
    assert_eq!(verdict.state, "healthy");
}

#[test]
fn terminal_instances_and_empty_frontier_are_healthy() {
    let mut book = QueueBook::new();
    packet(&mut book, "q-1", "owner1@rig", CREATED_MS, None);
    let packet = book.get(&qid("q-1")).expect("packet").clone();
    let now = Timestamp::from_millis(CREATED_MS + 10 * STUCK_THRESHOLD_SECONDS * 1000);
    assert_eq!(
        evaluate_deadline(
            &instance("done", Status::Completed, &["q-1"]),
            std::slice::from_ref(&packet),
            now
        )
        .state,
        "healthy"
    );
    assert_eq!(
        evaluate_deadline(&instance("empty", Status::Active, &[]), &[], now).state,
        "healthy"
    );
    assert_eq!(
        evaluate_deadline(
            &instance("other", Status::Active, &["some-other"]),
            &[packet],
            now
        )
        .state,
        "healthy"
    );
}

#[test]
fn claimed_then_unclaimed_is_overdue_unclaimed_and_a_fresh_packet_is_healthy() {
    let mut book = QueueBook::new();
    packet(&mut book, "q-1", "worker@rig", CREATED_MS, Some("fast"));
    let id = qid("q-1");
    let worker = session("worker@rig");
    book.claim(
        &id,
        &worker,
        Timestamp::from_millis(CREATED_MS + 1_000),
        None,
    )
    .expect("claim");
    book.unclaim(
        &id,
        &worker,
        "seat died mid-claim",
        Timestamp::from_millis(CREATED_MS + 2_000),
    )
    .expect("unclaim");
    let unclaimed = book.get(&id).expect("packet").clone();
    assert!(unclaimed.claimed_at.is_none());
    assert!(unclaimed.closure_required_at.is_none());
    let row = instance("inst", Status::Active, &["q-1"]);
    let past = Timestamp::from_millis(CREATED_MS + STUCK_THRESHOLD_SECONDS * 1000 + 1_000);
    let verdict = evaluate_deadline(&row, &[unclaimed], past);
    assert_eq!(verdict.state, "overdue-unclaimed");
    assert_eq!(verdict.evidence.expect("evidence").anchor, "created_at");
    packet(&mut book, "q-next", "worker@rig", past.millis(), None);
    let fresh = book.get(&qid("q-next")).expect("next").clone();
    let advanced = instance("inst", Status::Active, &["q-next"]);
    assert_eq!(
        evaluate_deadline(&advanced, &[fresh], past).state,
        "healthy"
    );
}

#[test]
fn exceeds_max_hops_helper() {
    assert!(!exceeds_max_hops(999, MAX_HOPS_BASELINE_V1, None));
    assert!(!exceeds_max_hops(2, 0, Some(3)));
    assert!(exceeds_max_hops(3, 0, Some(3)));
    assert!(!exceeds_max_hops(3, 3, Some(3)));
}

#[test]
fn keepalive_and_stuck_messages_match_the_sha256_fixtures() {
    let plain = keepalive_message("wf", "1", "inst-active", "active", &[]);
    assert_eq!(plain, include_str!("fixtures/watchdog/keepalive.txt"));
    let others = [
        "owner2@rig".to_owned(),
        "creator@rig".to_owned(),
        "registered@rig".to_owned(),
    ];
    let listed = keepalive_message("wf", "1", "inst-active", "active", &others);
    assert_eq!(
        listed,
        include_str!("fixtures/watchdog/keepalive-others.txt")
    );
    let mut book = QueueBook::new();
    packet(&mut book, "q-1", "owner1@rig", CREATED_MS, None);
    let row = instance("inst-active", Status::Active, &["q-1"]);
    let packet = book.get(&qid("q-1")).expect("packet").clone();
    let now = Timestamp::from_millis(CREATED_MS + 8 * 60 * 60 * 1000);
    let verdict = evaluate_deadline(&row, &[packet], now);
    let evidence = verdict.evidence.expect("stuck");
    assert_eq!(
        stuck_message(&evidence),
        include_str!("fixtures/watchdog/stuck.txt")
    );
    assert_eq!(
        nudge_pointer(&qid("q-1")),
        include_str!("fixtures/watchdog/pointer.txt")
    );
    let ledger = include_str!("fixtures/labels.jsonl");
    for (file, bytes) in [
        ("watchdog/keepalive.txt", plain.as_str()),
        ("watchdog/keepalive-others.txt", listed.as_str()),
        ("watchdog/stuck.txt", stuck_message(&evidence).as_str()),
        ("watchdog/pointer.txt", nudge_pointer(&qid("q-1")).as_str()),
    ] {
        let digest = hex_encode(Sha256::digest(bytes.as_bytes()).as_slice());
        let needle = format!("\"file\":\"{file}\",\"sha256\":\"{digest}\"");
        assert!(ledger.contains(&needle), "{file} {digest}");
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        push_hex(&mut out, *byte);
        out
    })
}

fn push_hex(out: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let hi = usize::from(byte / 16);
    let lo = usize::from(byte % 16);
    if let (Some(hi), Some(lo)) = (HEX.get(hi), HEX.get(lo)) {
        out.push(char::from(*hi));
        out.push(char::from(*lo));
    }
}
