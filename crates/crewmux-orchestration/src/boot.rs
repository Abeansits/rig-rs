// Derived from openrig packages/daemon/src/domain/workflow-boot-sweep.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Restart sweep. It re-arms, re-nudges, and reports. It never calls `project`.

use crewmux_core::Timestamp;
use crewmux_messaging::{QueueBook, QueueState};
use crewmux_types::QItemId;

use crewmux_messaging::{NewItem, SessionRef};

use crate::deadline::{Verdict, evaluate_deadline};
use crate::dial::{self, DialChoice};
use crate::keepalive;
use crate::model::{Engine, Status};
use crate::world::World;

/// What the sweep did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sweep {
    /// Live instances visited.
    pub visited: usize,
    /// Keepalives created by this sweep.
    pub newly_armed: usize,
    /// Pending frontier packets that received a nudge.
    pub renudged: usize,
    /// Overdue verdicts.
    pub overdue: Vec<Verdict>,
    /// Closed frontier packets with no successor. Not advanced.
    pub inconsistent: Vec<String>,
}

/// Re-arm, re-nudge, and surface overdue work. Does not advance an instance.
pub fn boot_sweep(engine: &mut Engine, book: &mut QueueBook, world: &World, now: Timestamp) -> Sweep {
    let mut sweep = Sweep {
        visited: 0,
        newly_armed: 0,
        renudged: 0,
        overdue: Vec::new(),
        inconsistent: Vec::new(),
    };
    let ids: Vec<String> = engine
        .instances
        .iter()
        .filter(|instance| instance.status.is_live())
        .map(|instance| instance.id.as_str().to_owned())
        .collect();
    sweep.visited = ids.len();
    for id in ids {
        sweep_one(engine, book, world, &id, now, &mut sweep);
    }
    sweep
}

fn sweep_one(
    engine: &mut Engine,
    book: &mut QueueBook,
    world: &World,
    id: &str,
    now: Timestamp,
    sweep: &mut Sweep,
) {
    let Some(instance) = engine.instance(id) else {
        return;
    };
    let frontier: Vec<String> = instance.frontier.iter().map(|item| item.as_str().to_owned()).collect();
    let owner = frontier.first().cloned();
    if let Some(owner) = owner.as_deref().and_then(|packet| book.get(&qid(packet)).map(|item| item.destination.as_str().to_owned()))
        && keepalive::arm(engine, id, &owner)
    {
        sweep.newly_armed = sweep.newly_armed.saturating_add(1);
    }
    renudge(book, &frontier, now, sweep);
    let packets = frontier
        .iter()
        .filter_map(|packet| book.get(&qid(packet)).cloned())
        .collect::<Vec<_>>();
    if let Some(instance) = engine.instance(id) {
        let verdict = evaluate_deadline(instance, &packets, now);
        if verdict.state != "healthy" {
            sweep.overdue.push(verdict);
        }
        note_inconsistent(instance.status, &frontier, book, sweep);
    }
    drop(ensure_stuck(engine, book, world, id, now));
}

fn renudge(book: &mut QueueBook, frontier: &[String], now: Timestamp, sweep: &mut Sweep) {
    for packet in frontier {
        let id = qid(packet);
        let Some(item) = book.get(&id) else {
            continue;
        };
        if item.state == QueueState::Pending
            && item.last_nudge.is_none()
            && book
                .append_note(
                    &id,
                    "workflow.boot-nudge",
                    Some("workflow nudge"),
                    Some("recorded"),
                    now,
                )
                .is_ok()
        {
            sweep.renudged = sweep.renudged.saturating_add(1);
        }
    }
}

fn note_inconsistent(status: Status, frontier: &[String], book: &QueueBook, sweep: &mut Sweep) {
    if !status.is_live() {
        return;
    }
    for packet in frontier {
        let Some(item) = book.get(&qid(packet)) else {
            continue;
        };
        let closed = matches!(
            item.state,
            QueueState::Done | QueueState::HandedOff | QueueState::Canceled | QueueState::Failed | QueueState::Denied
        );
        if closed && item.handed_off_to.is_none() {
            sweep.inconsistent.push(packet.clone());
        }
    }
}

fn ensure_stuck(
    engine: &Engine,
    book: &mut QueueBook,
    world: &World,
    id: &str,
    now: Timestamp,
) -> Option<DialChoice> {
    let instance = engine.instance(id)?;
    let tagged = format!("instance:{id}");
    if book.items().iter().any(|item| {
        item.tags.iter().any(|tag| tag == "stuck_overdue") && item.tags.iter().any(|tag| tag == &tagged)
    }) {
        return None;
    }
    let packets = instance
        .frontier
        .iter()
        .filter_map(|packet| book.get(packet).cloned())
        .collect::<Vec<_>>();
    let verdict = evaluate_deadline(instance, &packets, now);
    if verdict.state == "healthy" {
        return None;
    }
    let spec = engine
        .specs
        .get(&Engine::spec_key(&instance.workflow_name, &instance.workflow_version))
        .and_then(|cached| crewmux_spec::parse_workflow(&cached.yaml, &cached.source_path).ok())?;
    let choice = dial::route_exception(&spec, "stuck_overdue", world);
    let source = SessionRef::new("workflow@kernel").ok()?;
    let destination = SessionRef::new(&choice.session).ok()?;
    drop(book.create(
        NewItem {
            id: None,
            source,
            destination,
            body: format!("stuck_overdue {id} {}", verdict.state),
            priority: 0,
            tier: Some(choice.tier.to_owned()),
            tags: vec![
                "workflow".to_owned(),
                "exception".to_owned(),
                "stuck_overdue".to_owned(),
                tagged,
            ],
            handed_off_from: None,
            last_nudge: None,
            explicit_human: choice.tier == "human-gate",
            fingerprint: String::new(),
        },
        now,
    ));
    Some(choice)
}

fn qid(text: &str) -> QItemId {
    QItemId::new(text).unwrap_or_else(|_| QItemId::generate())
}
