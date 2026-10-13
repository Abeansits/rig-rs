//! Serial-scribe behavior: validation, projection, deadlines, and the boot sweep.

use std::path::PathBuf;

use crewmux_core::Timestamp;
use crewmux_messaging::{QueueBook, QueueState};
use crewmux_orchestration::{
    Engine, Exit, InstantiateRequest, Phase, ProjectCtx, ProjectRequest, RolePick, Runtime,
    SeatCandidate, Status, World, boot_sweep, evaluate_deadline, select_role_seat,
};
use crewmux_spec::parse_workflow;
use crewmux_types::QItemId;
use sha2::{Digest, Sha256};

const LINEAR: &str = include_str!("../fixtures/workflow-specs/linear-build.yaml");

fn now(millis: i64) -> Timestamp {
    Timestamp::from_millis(millis)
}

fn world() -> World {
    World::default()
}

fn request<'a>(yaml: &'a str, path: &'a str) -> InstantiateRequest<'a> {
    InstantiateRequest {
        yaml,
        path,
        objective: "ship",
        created_by: "orch@acme-flow",
        entry_owner: None,
        rig: None,
    }
}

fn project_at(
    engine: &Engine,
    book: &QueueBook,
    step: (&str, &str, Exit),
    at: Timestamp,
    crash: Option<Phase>,
) -> Result<(Engine, QueueBook, crewmux_orchestration::Projection), crewmux_orchestration::Error> {
    let (instance, packet, exit) = step;
    let inventory = world();
    let mut observe = || {};
    let mut ctx = ProjectCtx {
        world: &inventory,
        now: at,
        crash,
        observe: &mut observe,
    };
    Runtime::project(
        engine,
        book,
        &ProjectRequest {
            instance_id: instance.to_owned(),
            packet_id: packet.to_owned(),
            exit,
            actor: "flow-planner@acme-flow".to_owned(),
            result_note: None,
            blocked_on: None,
            evidence: None,
            next_owner: None,
            expected_version: None,
        },
        &mut ctx,
    )
}

#[test]
fn builtins_match_the_sha256_ledger() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/workflow-specs");
    let ledger = std::fs::read_to_string(dir.join("sha256.jsonl")).expect("ledger");
    for line in ledger.lines() {
        let row: serde_json::Value = serde_json::from_str(line).expect("row");
        let file = row["file"].as_str().expect("file");
        let bytes = std::fs::read(dir.join(file)).expect("bytes");
        let digest = hex(Sha256::digest(&bytes).as_slice());
        assert_eq!(digest, row["sha256"].as_str().expect("hash"), "{file}");
        let text = std::str::from_utf8(&bytes).expect("utf8");
        parse_workflow(text, file).expect("parse");
        let report = Runtime::validate_text(text, file).expect("validate");
        assert!(report.ok, "{file}: {:?}", report.issues);
        let expected = row["bytes"].as_u64().expect("len");
        assert_eq!(u64::try_from(bytes.len()).expect("len"), expected);
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut acc, byte| {
        let high = u32::from(byte / 16);
        let low = u32::from(byte % 16);
        acc.push(char::from_digit(high, 16).unwrap_or('0'));
        acc.push(char::from_digit(low, 16).unwrap_or('0'));
        acc
    })
}

#[test]
fn linear_build_validates_and_projects_to_done() {
    let report = Runtime::validate_text(LINEAR, "linear-build.yaml").expect("validate");
    assert!(report.ok, "{:?}", report.issues);
    let (engine, book, created) = Runtime::instantiate(
        &Engine::default(),
        &QueueBook::new(),
        &request(LINEAR, "linear-build.yaml"),
        &world(),
        now(1_000),
    )
    .expect("instantiate");
    assert_eq!(created.entry_owner, "flow-planner@acme-flow");
    assert_eq!(
        engine.instance(&created.instance_id).expect("row").version,
        1
    );
    let published = engine.clone();
    let (engine, book, first) = project_at(
        &engine,
        &book,
        (&created.instance_id, &created.entry_qitem, Exit::Handoff),
        now(2_000),
        None,
    )
    .expect("plan");
    assert_eq!(
        published
            .instance(&created.instance_id)
            .expect("same")
            .version,
        1
    );
    assert_eq!(
        first.events,
        [
            "queue.updated",
            "workflow.step_closed",
            "queue.created",
            "workflow.next_qitem_projected",
        ]
    );
    let packet = first.next_qitem.expect("build");
    let (engine, book, second) = project_at(
        &engine,
        &book,
        (&created.instance_id, &packet, Exit::Handoff),
        now(3_000),
        None,
    )
    .expect("build");
    drop(second);
    let packet = book
        .items()
        .into_iter()
        .find(|item| item.state == QueueState::Pending)
        .expect("prove")
        .id;
    let (engine, book, done) = project_at(
        &engine,
        &book,
        (&created.instance_id, packet.as_str(), Exit::Done),
        now(4_000),
        None,
    )
    .expect("done");
    assert!(done.events.contains(&"workflow.completed"));
    let instance = engine.instance(&created.instance_id).expect("done row");
    assert_eq!(instance.status, Status::Completed);
    assert!(!engine.keepalives.iter().any(|job| job.armed));
    assert_eq!(
        book.get(&QItemId::new(&created.entry_qitem).expect("id"))
            .expect("entry")
            .state,
        QueueState::HandedOff
    );
}

#[test]
fn crash_and_observer_leave_the_published_pair() {
    let (engine, book, created) = Runtime::instantiate(
        &Engine::default(),
        &QueueBook::new(),
        &request(LINEAR, "linear-build.yaml"),
        &world(),
        now(1_000),
    )
    .expect("instantiate");
    let inventory = world();
    let mut saw = false;
    let mut ctx = ProjectCtx {
        world: &inventory,
        now: now(2_000),
        crash: Some(Phase::AfterClose),
        observe: &mut || {
            saw = true;
            assert_eq!(
                engine
                    .instance(&created.instance_id)
                    .expect("row")
                    .hop_count,
                0
            );
        },
    };
    let err = Runtime::project(
        &engine,
        &book,
        &ProjectRequest {
            instance_id: created.instance_id.clone(),
            packet_id: created.entry_qitem.clone(),
            exit: Exit::Handoff,
            actor: "flow-planner@acme-flow".to_owned(),
            result_note: None,
            blocked_on: None,
            evidence: None,
            next_owner: None,
            expected_version: None,
        },
        &mut ctx,
    )
    .expect_err("crash");
    assert!(saw);
    assert!(matches!(
        err,
        crewmux_orchestration::Error::Crashed("after-close")
    ));
    assert_eq!(
        engine.instance(&created.instance_id).expect("row").version,
        1
    );
    assert_eq!(
        book.get(&QItemId::new(&created.entry_qitem).expect("id"))
            .expect("item")
            .state,
        QueueState::Pending
    );
}

#[test]
fn continue_twice_writes_nothing_and_waiting_replay_is_absorbed() {
    let (engine, book, created) = Runtime::instantiate(
        &Engine::default(),
        &QueueBook::new(),
        &request(LINEAR, "linear-build.yaml"),
        &world(),
        now(1_000),
    )
    .expect("instantiate");
    let before = engine.clone();
    let first = Runtime::continue_view(&engine, &created.instance_id).expect("one");
    let second = Runtime::continue_view(&engine, &created.instance_id).expect("two");
    assert_eq!(first.trail_len, second.trail_len);
    assert_eq!(engine, before);
    let (engine, book, waiting) = project_at(
        &engine,
        &book,
        (&created.instance_id, &created.entry_qitem, Exit::Waiting),
        now(2_000),
        None,
    )
    .expect("wait");
    assert!(!waiting.absorbed);
    let version = engine.instance(&created.instance_id).expect("row").version;
    let (again, again_book, replay) = project_at(
        &engine,
        &book,
        (&created.instance_id, &created.entry_qitem, Exit::Waiting),
        now(3_000),
        None,
    )
    .expect("replay");
    assert!(replay.absorbed);
    assert_eq!(
        again.instance(&created.instance_id).expect("row").version,
        version
    );
    assert_eq!(again_book.epoch(), book.epoch());
}

#[test]
fn preferred_target_order_beats_liveness_and_codepoint_breaks_ties() {
    let yaml = "workflow:\n  id: pick\n  version: 1\n  roles:\n    planner:\n      preferred_targets: [dead@rig, live@rig]\n  steps:\n    - id: one\n      actor_role: planner\n";
    let (engine, _, created) = Runtime::instantiate(
        &Engine::default(),
        &QueueBook::new(),
        &request(yaml, "pick.yaml"),
        &world(),
        now(1_000),
    )
    .expect("instantiate");
    assert_eq!(created.entry_owner, "dead@rig");
    assert!(engine.instance(&created.instance_id).is_some());
    let pick: RolePick = select_role_seat(
        "planner",
        None,
        &[seat("b", "b@rig", 0), seat("a", "a@rig", 0)],
    );
    assert_eq!(pick.seat.as_deref(), Some("a@rig"));
}

fn seat(id: &str, coordinate: &str, pending: i64) -> SeatCandidate {
    SeatCandidate {
        logical_id: id.to_owned(),
        role: Some("planner".to_owned()),
        node_kind: "agent".to_owned(),
        lifecycle_state: "running".to_owned(),
        runtime: None,
        pending_work_count: pending,
        coordinate: Some(coordinate.to_owned()),
        raw_session_name: Some(coordinate.to_owned()),
    }
}

#[test]
fn unresolved_owner_does_not_mutate_and_graph_project_is_refused() {
    let bare = "workflow:\n  id: bare\n  version: 1\n  roles:\n    planner: {}\n  steps:\n    - id: one\n      actor_role: planner\n";
    let err = Runtime::instantiate(
        &Engine::default(),
        &QueueBook::new(),
        &request(bare, "bare.yaml"),
        &world(),
        now(1_000),
    )
    .expect_err("owner");
    assert!(matches!(
        err,
        crewmux_orchestration::Error::Rejected {
            code: "entry_owner_unresolved",
            ..
        }
    ));
    let graph = "workflow:\n  id: graph\n  version: 1\n  roles:\n    planner:\n      preferred_targets: [a@rig]\n  steps:\n    - id: one\n      actor_role: planner\n    - id: two\n      actor_role: planner\n      depends_on: [one]\n";
    let (engine, book, created) = Runtime::instantiate(
        &Engine::default(),
        &QueueBook::new(),
        &request(graph, "graph.yaml"),
        &world(),
        now(1_000),
    )
    .expect("graph instantiate");
    let err = project_at(
        &engine,
        &book,
        (&created.instance_id, &created.entry_qitem, Exit::Handoff),
        now(2_000),
        None,
    )
    .expect_err("graph");
    assert!(matches!(
        err,
        crewmux_orchestration::Error::Rejected {
            code: "frontier_binding_indeterminate",
            ..
        }
    ));
    assert_eq!(
        engine.instance(&created.instance_id).expect("row").version,
        1
    );
}

#[test]
fn fake_clock_deadline_and_boot_sweep_do_not_advance() {
    let (mut engine, mut book, created) = Runtime::instantiate(
        &Engine::default(),
        &QueueBook::new(),
        &request(LINEAR, "linear-build.yaml"),
        &world(),
        now(0),
    )
    .expect("instantiate");
    let instance = engine.instance(&created.instance_id).expect("row").clone();
    let packet = book
        .get(&QItemId::new(&created.entry_qitem).expect("id"))
        .expect("packet")
        .clone();
    let verdict = evaluate_deadline(&instance, &[packet], now(4 * 60 * 60 * 1000));
    assert_eq!(verdict.state, "overdue-unclaimed");
    let version = instance.version;
    let sweep = boot_sweep(&mut engine, &mut book, &world(), now(1_000));
    assert_eq!(sweep.visited, 1);
    assert_eq!(sweep.renudged, 0);
    assert_eq!(
        engine.instance(&created.instance_id).expect("row").version,
        version
    );
    assert_eq!(
        engine
            .instance(&created.instance_id)
            .expect("row")
            .hop_count,
        0
    );
}

#[test]
fn lost_nudge_is_reissued_once() {
    let (mut engine, book, created) = Runtime::instantiate(
        &Engine::default(),
        &QueueBook::new(),
        &request(LINEAR, "linear-build.yaml"),
        &world(),
        now(1_000),
    )
    .expect("instantiate");
    let version = engine.instance(&created.instance_id).expect("row").version;
    let mut quiet = book.items();
    for item in &mut quiet {
        item.last_nudge = None;
    }
    let mut book = QueueBook::restore(quiet, book.all_transitions());
    let sweep = boot_sweep(&mut engine, &mut book, &world(), now(2_000));
    assert_eq!(sweep.renudged, 1);
    assert_eq!(sweep.visited, 1);
    let id = QItemId::new(&created.entry_qitem).expect("id");
    let nudged = book.get(&id).expect("packet");
    assert!(nudged.last_nudge.is_some());
    assert!(
        book.all_transitions()
            .iter()
            .any(|row| row.note == "workflow.boot-nudge")
    );
    let again = boot_sweep(&mut engine, &mut book, &world(), now(3_000));
    assert_eq!(again.renudged, 0);
    let row = engine.instance(&created.instance_id).expect("row");
    assert_eq!(row.version, version);
    assert_eq!(row.hop_count, 0);
    assert_eq!(row.status, Status::Active);
}

#[test]
fn resume_of_a_failed_instance_reopens_one_packet() {
    let (engine, book, created) = Runtime::instantiate(
        &Engine::default(),
        &QueueBook::new(),
        &request(LINEAR, "linear-build.yaml"),
        &world(),
        now(1_000),
    )
    .expect("instantiate");
    let (engine, book, failed) = project_at(
        &engine,
        &book,
        (&created.instance_id, &created.entry_qitem, Exit::Failed),
        now(2_000),
        None,
    )
    .expect("fail");
    assert!(failed.events.contains(&"workflow.failed"));
    let failed_row = engine.instance(&created.instance_id).expect("failed");
    assert_eq!(failed_row.status, Status::Failed);
    let step = failed_row.current_step_id.clone();
    let (opened, packets, packet) = Runtime::resume(
        &engine,
        &book,
        &created.instance_id,
        "orch@acme-flow",
        "orch@acme-flow",
        now(3_000),
    )
    .expect("resume");
    assert_eq!(
        engine.instance(&created.instance_id).expect("same").status,
        Status::Failed
    );
    let row = opened.instance(&created.instance_id).expect("resumed");
    assert_eq!(row.status, Status::Active);
    assert_eq!(row.resume_count, 1);
    assert_eq!(row.current_step_id, step);
    assert_eq!(row.frontier.len(), 1);
    assert_ne!(packet, created.entry_qitem);
    let id = QItemId::new(&packet).expect("id");
    let item = packets.get(&id).expect("packet");
    assert_eq!(item.state, QueueState::Pending);
    assert_eq!(item.destination.as_str(), "orch@acme-flow");
    assert!(opened.keepalives.iter().any(|job| job.armed));
    let err = Runtime::resume(
        &opened,
        &packets,
        &created.instance_id,
        "orch@acme-flow",
        "orch@acme-flow",
        now(4_000),
    )
    .expect_err("second");
    assert!(matches!(
        err,
        crewmux_orchestration::Error::Rejected {
            code: "instance_not_resumable",
            ..
        }
    ));
}

#[test]
fn duplicate_step_id_is_step_id_duplicate() {
    let yaml = "workflow:\n  id: dup\n  version: 1\n  roles:\n    planner:\n      preferred_targets: [a@rig]\n  steps:\n    - id: one\n      actor_role: planner\n    - id: one\n      actor_role: planner\n";
    let report = Runtime::validate_text(yaml, "dup.yaml").expect("report");
    assert!(!report.ok);
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.code == "step_id_duplicate")
    );
}
