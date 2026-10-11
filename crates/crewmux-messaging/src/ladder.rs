// Derived from openrig packages/daemon/src/domain/queue-wake-ladder.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! The row carries the obligation exactly once. The wake is at-least-once.
//! Ladder position is derived from the transition log, so a restart cannot reset the count.

use std::time::Duration;

use crewmux_core::Timestamp;
use crewmux_types::QItemId;

use crate::model::{QueueItem, Transition};
use crate::nudge::nudge_pointer;
use crate::session::SessionRef;
use crate::state::QueueState;

/// `wake-attempt:`
pub const WAKE_ATTEMPT_PREFIX: &str = "wake-attempt:";
/// `escalation-rung:`
pub const ESCALATION_RUNG_PREFIX: &str = "escalation-rung:";
/// `ladder-exhausted:`
pub const LADDER_EXHAUSTED_PREFIX: &str = "ladder-exhausted:";
/// `ladder-suspend:`
pub const LADDER_SUSPEND_PREFIX: &str = "ladder-suspend:";
/// `ladder-resume:`
pub const LADDER_RESUME_PREFIX: &str = "ladder-resume:";

/// `queue.wake_human_rung`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WakeHumanRung {
    /// Nudge human seats.
    Always,
    /// Nudge a human seat only when the item was explicitly addressed there.
    ExplicitOnly,
}

/// Retry cap, interval, and the unconfirmed window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WakeConfig {
    /// Failed nudges before the orchestrator rung. TypeScript default is 3.
    pub retry_cap: u8,
    /// Minimum gap between attempts. TypeScript default is 300s.
    pub retry_interval: Duration,
    /// Delivered-but-unconfirmed items wait this long before escalation.
    pub unconfirmed_window: Duration,
    /// Human-seat policy.
    pub human_rung: WakeHumanRung,
}

impl Default for WakeConfig {
    fn default() -> Self {
        Self {
            retry_cap: 3,
            retry_interval: Duration::from_secs(300),
            unconfirmed_window: Duration::from_mins(30),
            human_rung: WakeHumanRung::ExplicitOnly,
        }
    }
}

/// What one tick wants the caller to record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WakeAction {
    /// Send the pointer text to `target`.
    Nudge {
        /// Item.
        qitem: QItemId,
        /// Destination session.
        target: SessionRef,
    },
    /// One rung. Orchestrator escalations share `aggregate_key`.
    Escalate {
        /// Which rung.
        rung: Rung,
        /// Destination session. Callers emit one aggregate row per key.
        aggregate_key: String,
    },
    /// A suspend marker is open.
    Suspend,
    /// The operator rung is already recorded.
    Exhausted,
}

/// Escalation step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rung {
    /// Retry number, 1-based.
    Retry(u8),
    /// Destination orchestrator, once per destination.
    DestinationOrchestrator,
    /// Operator surface.
    Operator,
}

/// Pure ladder. `tick` does not send and does not store.
#[derive(Clone, Debug)]
pub struct WakeLadder {
    cfg: WakeConfig,
}

impl WakeLadder {
    /// `cfg` supplies the cap, the interval, and the human-seat policy.
    #[must_use]
    pub const fn new(cfg: WakeConfig) -> Self {
        Self { cfg }
    }

    /// Next actions for one derived view.
    #[must_use]
    pub fn tick(&self, view: &LadderView, now: Timestamp) -> Vec<WakeAction> {
        let Some(action) = self.one(view, now) else {
            return Vec::new();
        };
        vec![action]
    }

    fn one(&self, view: &LadderView, now: Timestamp) -> Option<WakeAction> {
        if !view.eligible() {
            return None;
        }
        if self.skip_human(view) {
            return None;
        }
        let pos = derive(&view.notes);
        if marked(&pos, MARK_EXHAUSTED) {
            return None;
        }
        if marked(&pos, MARK_SUSPENDED) {
            return Some(WakeAction::Suspend);
        }
        match classify(view.last_nudge_result.as_deref()) {
            Some(Mode::Unconfirmed) => self.unconfirmed(view, &pos, now),
            Some(Mode::Failed) => self.failed(view, &pos, now),
            None => None,
        }
    }

    fn skip_human(&self, view: &LadderView) -> bool {
        view.destination.is_human_seat()
            && self.cfg.human_rung == WakeHumanRung::ExplicitOnly
            && !view.explicit_human
    }

    fn unconfirmed(
        &self,
        view: &LadderView,
        pos: &LadderPos,
        now: Timestamp,
    ) -> Option<WakeAction> {
        if view.claimed_at.is_some() {
            return None;
        }
        let due = view.created_at.saturating_add(self.cfg.unconfirmed_window);
        if now < due {
            return None;
        }
        Some(escalate(view, pos))
    }

    fn failed(&self, view: &LadderView, pos: &LadderPos, now: Timestamp) -> Option<WakeAction> {
        if pos.attempts < self.cfg.retry_cap {
            return self.retry(view, pos, now);
        }
        Some(escalate(view, pos))
    }

    fn retry(&self, view: &LadderView, pos: &LadderPos, now: Timestamp) -> Option<WakeAction> {
        if let Some(last) = pos.last_marker {
            let due = last.saturating_add(self.cfg.retry_interval);
            if now < due {
                return None;
            }
        }
        Some(WakeAction::Nudge {
            qitem: view.qitem.clone(),
            target: view.destination.clone(),
        })
    }
}

/// Collapse orchestrator escalations that share a destination.
#[must_use]
pub fn dedup_escalations(actions: Vec<WakeAction>) -> Vec<WakeAction> {
    let mut seen = Vec::<String>::new();
    let mut out = Vec::new();
    for action in actions {
        if let WakeAction::Escalate {
            rung: Rung::DestinationOrchestrator,
            aggregate_key,
        } = &action
        {
            if seen.iter().any(|key| key == aggregate_key) {
                continue;
            }
            seen.push(aggregate_key.clone());
        }
        out.push(action);
    }
    out
}

/// Sentence stored on an orchestrator rung.
#[must_use]
pub fn escalation_reason(view: &LadderView, cfg: &WakeConfig, now: Timestamp) -> String {
    if classify(view.last_nudge_result.as_deref()) == Some(Mode::Unconfirmed) {
        "unconfirmed past window".to_owned()
    } else {
        format!(
            "wake failed {} times over {} min",
            cfg.retry_cap,
            elapsed_min(view, now)
        )
    }
}

/// Note the caller appends after it performs `action`.
#[must_use]
pub fn transition_note(action: &WakeAction, attempt: u8, cap: u8, reason: &str) -> String {
    match action {
        WakeAction::Nudge { .. } => format!("{WAKE_ATTEMPT_PREFIX} {attempt}/{cap} outcome=failed"),
        WakeAction::Escalate {
            rung: Rung::DestinationOrchestrator,
            ..
        } => format!("{ESCALATION_RUNG_PREFIX} orchestrator {reason}"),
        WakeAction::Escalate {
            rung: Rung::Operator,
            ..
        } => format!("{ESCALATION_RUNG_PREFIX} operator {reason}"),
        WakeAction::Escalate {
            rung: Rung::Retry(n),
            ..
        } => format!("{WAKE_ATTEMPT_PREFIX} {n}/{cap} outcome=failed"),
        WakeAction::Suspend => format!("{LADDER_SUSPEND_PREFIX} suspended"),
        WakeAction::Exhausted => format!("{LADDER_EXHAUSTED_PREFIX} operator rung recorded"),
    }
}

/// Pointer text for a nudge. Never includes `body`.
#[must_use]
pub fn nudge_body(id: &QItemId, body: &str) -> String {
    let _ = body;
    nudge_pointer(id)
}

/// One item, reduced to what the ladder reads.
#[derive(Clone, Debug)]
pub struct LadderView {
    /// Item.
    pub qitem: QItemId,
    /// Where the nudge would go.
    pub destination: SessionRef,
    /// Obligation. Used only to prove the nudge omits it.
    pub body: String,
    /// State.
    pub state: QueueState,
    /// Set on a baton created by handoff.
    pub handed_off_from: Option<QItemId>,
    /// Last nudge outcome.
    pub last_nudge_result: Option<String>,
    /// Created.
    pub created_at: Timestamp,
    /// Set once someone claimed the baton.
    pub claimed_at: Option<Timestamp>,
    /// Explicit human address.
    pub explicit_human: bool,
    /// `(at, note)` pairs, oldest first.
    pub notes: Vec<(Timestamp, String)>,
}

impl LadderView {
    /// Project a stored row and its log.
    #[must_use]
    pub fn from_item(item: &QueueItem, transitions: &[Transition]) -> Self {
        Self {
            qitem: item.id.clone(),
            destination: item.destination.clone(),
            body: item.body.clone(),
            state: item.state,
            handed_off_from: item.handed_off_from.clone(),
            last_nudge_result: item.last_nudge.as_ref().map(|nudge| nudge.result.clone()),
            created_at: item.created_at,
            claimed_at: item.claimed_at,
            explicit_human: item.explicit_human,
            notes: transitions
                .iter()
                .map(|row| (row.at, row.note.clone()))
                .collect(),
        }
    }

    fn eligible(&self) -> bool {
        self.handed_off_from.is_some()
            && matches!(self.state, QueueState::Pending | QueueState::InProgress)
    }
}

const MARK_ORCHESTRATOR: u8 = 1;
const MARK_OPERATOR: u8 = 2;
const MARK_EXHAUSTED: u8 = 4;
const MARK_SUSPENDED: u8 = 8;

#[derive(Clone, Copy)]
struct LadderPos {
    attempts: u8,
    marks: u8,
    last_marker: Option<Timestamp>,
}

fn marked(pos: &LadderPos, bit: u8) -> bool {
    pos.marks & bit != 0
}

fn raise(pos: &mut LadderPos, bit: u8) {
    pos.marks |= bit;
}

fn escalate(view: &LadderView, pos: &LadderPos) -> WakeAction {
    let aggregate_key = view.destination.as_str().to_owned();
    if !marked(pos, MARK_ORCHESTRATOR) {
        return WakeAction::Escalate {
            rung: Rung::DestinationOrchestrator,
            aggregate_key,
        };
    }
    if !marked(pos, MARK_OPERATOR) {
        return WakeAction::Escalate {
            rung: Rung::Operator,
            aggregate_key,
        };
    }
    WakeAction::Exhausted
}

fn derive(notes: &[(Timestamp, String)]) -> LadderPos {
    let mut pos = LadderPos {
        attempts: 0,
        marks: 0,
        last_marker: None,
    };
    let mut suspends: u32 = 0;
    let mut resumes: u32 = 0;
    for (at, note) in notes {
        if note.starts_with(WAKE_ATTEMPT_PREFIX) {
            pos.attempts = pos.attempts.saturating_add(1);
            pos.last_marker = Some(*at);
        }
        if note.starts_with(ESCALATION_RUNG_PREFIX) && note.contains("orchestrator") {
            raise(&mut pos, MARK_ORCHESTRATOR);
            pos.last_marker = Some(*at);
        }
        if note.starts_with(ESCALATION_RUNG_PREFIX) && note.contains("operator") {
            raise(&mut pos, MARK_OPERATOR);
            pos.last_marker = Some(*at);
        }
        if note.starts_with(LADDER_EXHAUSTED_PREFIX) {
            raise(&mut pos, MARK_EXHAUSTED);
        }
        if note.starts_with(LADDER_SUSPEND_PREFIX) {
            suspends = suspends.saturating_add(1);
        }
        if note.starts_with(LADDER_RESUME_PREFIX) {
            resumes = resumes.saturating_add(1);
        }
    }
    if suspends > resumes {
        raise(&mut pos, MARK_SUSPENDED);
    }
    pos
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Failed,
    Unconfirmed,
}

fn classify(result: Option<&str>) -> Option<Mode> {
    let result = result?;
    if result.starts_with("failed:") || result.starts_with("retained:") {
        return Some(Mode::Failed);
    }
    if result == "delivered-ack-pending"
        || result.starts_with("indeterminate:")
        || result.starts_with("gateway-owned:")
    {
        return Some(Mode::Unconfirmed);
    }
    None
}

fn elapsed_min(view: &LadderView, now: Timestamp) -> u64 {
    now.saturating_duration_since(view.created_at).as_secs() / 60
}
