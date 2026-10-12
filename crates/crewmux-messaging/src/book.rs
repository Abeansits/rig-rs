//! In-memory queue book. Each mutation builds a draft and swaps it in at the end.
//!
//! A crash hook returns before the swap, so the visible book is the before state.

use std::collections::BTreeMap;

use crewmux_core::Timestamp;
use crewmux_types::QItemId;

use crate::closure::{closure_required_at, validate_closure};
use crate::error::Error;
use crate::machine::admit;
use crate::model::{Actor, NewItem, QueueItem, Transition, Update};
use crate::nudge::obligation_fingerprint;
use crate::session::SessionRef;
use crate::state::QueueState;

/// The queue, its transition log, and the crash hook.
#[derive(Clone, Debug, Default)]
pub struct QueueBook {
    items: BTreeMap<QItemId, QueueItem>,
    transitions: Vec<Transition>,
    next_transition: u64,
    /// When set, the named step returns [`Error::Crashed`] and discards the draft.
    crash_at: Option<&'static str>,
    /// Increases on every committed mutation. A persist actor keeps the newest.
    epoch: u64,
}

struct LogLine {
    qitem: QItemId,
    from: QueueState,
    to: QueueState,
    note: String,
    at: Timestamp,
    actor: Actor,
}

impl QueueBook {
    /// An empty book.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fail the next mutating call when it reaches `point`, before the swap.
    pub fn arm_crash(&mut self, point: &'static str) {
        self.crash_at = Some(point);
    }

    /// Clear the hook.
    pub fn clear_crash(&mut self) {
        self.crash_at = None;
    }

    /// Commit counter. Zero on a restored book until the next mutation.
    #[must_use]
    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Rebuild a book from a persisted snapshot. The crash hook is clear.
    #[must_use]
    pub fn restore(items: Vec<QueueItem>, transitions: Vec<Transition>) -> Self {
        let next_transition = transitions.iter().map(|row| row.id).max().unwrap_or(0);
        let mut map = BTreeMap::new();
        for item in items {
            map.insert(item.id.clone(), item);
        }
        Self {
            items: map,
            transitions,
            next_transition,
            crash_at: None,
            epoch: 0,
        }
    }

    /// Every transition, oldest first.
    #[must_use]
    pub fn all_transitions(&self) -> Vec<Transition> {
        self.transitions.clone()
    }

    /// Every item, oldest id first.
    #[must_use]
    pub fn items(&self) -> Vec<QueueItem> {
        self.items.values().cloned().collect()
    }

    /// One item.
    #[must_use]
    pub fn get(&self, id: &QItemId) -> Option<&QueueItem> {
        self.items.get(id)
    }

    /// Transitions for one item, in order.
    #[must_use]
    pub fn transitions(&self, id: &QItemId) -> Vec<Transition> {
        self.transitions
            .iter()
            .filter(|row| &row.qitem == id)
            .cloned()
            .collect()
    }

    /// Insert a pending item and a `created` transition.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Crashed`] when the hook is `before-commit`.
    pub fn create(&mut self, new: NewItem, now: Timestamp) -> Result<QueueItem, Error> {
        let mut draft = self.clone();
        let item = draft.insert_new(new, now);
        self.swap(draft, "before-commit")?;
        Ok(item)
    }

    /// Apply a state write.
    ///
    /// # Errors
    ///
    /// Returns a typed rejection, [`Error::NotFound`], or [`Error::Crashed`].
    pub fn update(
        &mut self,
        id: &QItemId,
        update: Update,
        now: Timestamp,
    ) -> Result<Transition, Error> {
        let mut draft = self.clone();
        let transition = draft.write(id, update, now)?;
        self.swap(draft, "before-commit")?;
        Ok(transition)
    }

    /// Pending or blocked → in progress, for the destination session.
    ///
    /// # Errors
    ///
    /// Returns [`Error::AlreadyClaimed`] for a second claimant unless `force`
    /// carries a non-empty reason.
    pub fn claim(
        &mut self,
        id: &QItemId,
        by: &SessionRef,
        now: Timestamp,
        force: Option<&str>,
    ) -> Result<QueueItem, Error> {
        let mut draft = self.clone();
        draft.claim_draft(id, by, now, force)?;
        self.swap(draft, "before-commit")?;
        self.items.get(id).cloned().ok_or_else(|| missing(id))
    }

    /// In progress → pending.
    ///
    /// # Errors
    ///
    /// Returns a typed rejection when the row is not in progress.
    pub fn unclaim(
        &mut self,
        id: &QItemId,
        by: &SessionRef,
        reason: &str,
        now: Timestamp,
    ) -> Result<QueueItem, Error> {
        let item = self.must(id)?;
        if item.state != QueueState::InProgress {
            return Err(Error::Rejected {
                code: "qitem_not_in_progress",
                message: format!(
                    "qitem {id} is in state {}; only in-progress can be unclaimed",
                    item.state.as_str()
                ),
            });
        }
        let update = Update {
            state: Some(QueueState::Pending),
            actor: Actor::Session(by.clone()),
            note: format!("unclaimed: {reason}"),
            closure_reason: None,
            closure_target: None,
            reopen: false,
            blocked_on: None,
        };
        self.update(id, update, now)?;
        self.must(id)
    }

    /// Close the source as `handed-off` and insert the successor. One swap.
    ///
    /// # Errors
    ///
    /// Returns a closure or terminal error, or [`Error::Crashed`] at
    /// `after-source`, `after-successor`, or `before-commit`.
    pub fn handoff(
        &mut self,
        id: &QItemId,
        to: &SessionRef,
        actor: &SessionRef,
        now: Timestamp,
        complete: bool,
    ) -> Result<(QueueItem, QueueItem), Error> {
        let mut draft = self.clone();
        let closed_state = if complete {
            QueueState::Done
        } else {
            QueueState::HandedOff
        };
        let update = Update {
            state: Some(closed_state),
            actor: Actor::Session(actor.clone()),
            note: if complete {
                "handoff-and-complete".to_owned()
            } else {
                "handoff".to_owned()
            },
            closure_reason: Some("handed_off_to".to_owned()),
            closure_target: Some(to.as_str().to_owned()),
            reopen: false,
            blocked_on: None,
        };
        draft.write(id, update, now)?;
        self.gate(&draft, "after-source")?;
        let successor = draft.successor_of(id, to, now)?;
        self.gate(&draft, "after-successor")?;
        let closed = draft.must(id)?;
        self.swap(draft, "before-commit")?;
        Ok((closed, successor))
    }

    /// Point the row at a new destination and clear the claim.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`] or a crash.
    pub fn fallback(
        &mut self,
        id: &QItemId,
        to: &SessionRef,
        actor: &SessionRef,
        now: Timestamp,
    ) -> Result<QueueItem, Error> {
        let mut draft = self.clone();
        let mut item = draft.must(id)?;
        let from = item.destination.clone();
        item.destination = to.clone();
        item.state = QueueState::Pending;
        item.claimed_at = None;
        item.claimed_by = None;
        item.updated_at = now;
        draft.items.insert(id.clone(), item);
        draft.push(LogLine {
            qitem: id.clone(),
            from: QueueState::Pending,
            to: QueueState::Pending,
            note: format!("qitem.fallback_routed {from} -> {to}"),
            at: now,
            actor: Actor::Session(actor.clone()),
        });
        self.swap(draft, "before-commit")?;
        self.must(id)
    }

    /// In-progress rows past `closure.required_at`.
    #[must_use]
    pub fn overdue(&self, now: Timestamp) -> Vec<QueueItem> {
        self.items
            .values()
            .filter(|item| item.state == QueueState::InProgress)
            .filter(|item| item.closure_required_at.is_some_and(|due| due <= now))
            .cloned()
            .collect()
    }

    /// Pending rows whose last nudge failed and that were never claimed.
    #[must_use]
    pub fn undelivered(&self) -> Vec<QueueItem> {
        self.items
            .values()
            .filter(|item| item.state == QueueState::Pending && item.claimed_at.is_none())
            .filter(|item| {
                item.last_nudge
                    .as_ref()
                    .is_some_and(|nudge| nudge.result.starts_with("failed:"))
            })
            .cloned()
            .collect()
    }

    /// Terminal rows whose `updated_at` is older than `cutoff`.
    #[must_use]
    pub fn retention_ids(&self, cutoff: Timestamp) -> Vec<QItemId> {
        self.items
            .values()
            .filter(|item| item.state.is_terminal() && item.updated_at < cutoff)
            .map(|item| item.id.clone())
            .collect()
    }

    /// Append a ladder note without copying the item body.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`].
    pub fn append_note(
        &mut self,
        id: &QItemId,
        note: &str,
        nudge_text: Option<&str>,
        nudge_result: Option<&str>,
        now: Timestamp,
    ) -> Result<Transition, Error> {
        let mut draft = self.clone();
        let mut item = draft.must(id)?;
        if let (Some(text), Some(result)) = (nudge_text, nudge_result) {
            item.last_nudge = Some(crate::model::NudgeRecord {
                at: now,
                result: result.to_owned(),
                text: text.to_owned(),
            });
        }
        item.updated_at = now;
        let state = item.state;
        draft.items.insert(id.clone(), item);
        let transition = draft.push(LogLine {
            qitem: id.clone(),
            from: state,
            to: state,
            note: note.to_owned(),
            at: now,
            actor: Actor::System,
        });
        self.swap(draft, "before-commit")?;
        Ok(transition)
    }

    fn insert_new(&mut self, new: NewItem, now: Timestamp) -> QueueItem {
        let id = new.id.unwrap_or_else(QItemId::generate);
        let fingerprint = if new.fingerprint.is_empty() {
            obligation_fingerprint(new.source.as_str(), new.destination.as_str(), &new.body)
        } else {
            new.fingerprint
        };
        let item = QueueItem {
            id: id.clone(),
            source: new.source.clone(),
            destination: new.destination,
            state: QueueState::Pending,
            priority: new.priority,
            tier: new.tier,
            tags: new.tags,
            blocked_on: None,
            handed_off_to: None,
            handed_off_from: new.handed_off_from,
            expires_at: None,
            closure: None,
            closure_required_at: None,
            claimed_at: None,
            claimed_by: None,
            last_nudge: new.last_nudge,
            body: new.body,
            fingerprint,
            explicit_human: new.explicit_human,
            created_at: now,
            updated_at: now,
        };
        self.items.insert(id.clone(), item.clone());
        self.push(LogLine {
            qitem: id,
            from: QueueState::Pending,
            to: QueueState::Pending,
            note: "created".to_owned(),
            at: now,
            actor: Actor::Session(new.source),
        });
        item
    }

    fn write(&mut self, id: &QItemId, update: Update, now: Timestamp) -> Result<Transition, Error> {
        let mut item = self.must(id)?;
        let from = item.state;
        let closure = admit(id, from, &update)?;
        let to = update.state.unwrap_or(from);
        item.state = to;
        item.updated_at = now;
        if let Some(mut closure) = closure {
            closure.required_at = item.closure.as_ref().and_then(|prior| prior.required_at);
            item.closure = Some(closure);
        }
        if to == QueueState::Blocked {
            item.blocked_on = update.blocked_on.clone().or(item.blocked_on);
        }
        if to == QueueState::Pending {
            item.claimed_at = None;
            item.claimed_by = None;
            item.closure_required_at = None;
        }
        self.items.insert(id.clone(), item);
        Ok(self.push(LogLine {
            qitem: id.clone(),
            from,
            to,
            note: update.note,
            at: now,
            actor: update.actor,
        }))
    }

    fn claim_draft(
        &mut self,
        id: &QItemId,
        by: &SessionRef,
        now: Timestamp,
        force: Option<&str>,
    ) -> Result<(), Error> {
        let item = self.must(id)?;
        ensure_claimable(id, &item, by, force)?;
        let note = claim_note(force);
        self.write(id, claim_update(by, &note), now)?;
        self.stamp_claim(id, by, now)
    }

    fn push(&mut self, line: LogLine) -> Transition {
        self.next_transition = self.next_transition.saturating_add(1);
        let transition = Transition {
            id: self.next_transition,
            qitem: line.qitem,
            from: line.from,
            to: line.to,
            note: line.note,
            at: line.at,
            actor: line.actor,
        };
        self.transitions.push(transition.clone());
        transition
    }

    fn stamp_claim(&mut self, id: &QItemId, by: &SessionRef, now: Timestamp) -> Result<(), Error> {
        let mut item = self.must(id)?;
        item.claimed_at = Some(now);
        item.claimed_by = Some(by.clone());
        item.blocked_on = None;
        item.closure_required_at = closure_required_at(now, item.tier.as_deref());
        self.items.insert(id.clone(), item);
        Ok(())
    }

    fn successor_of(
        &mut self,
        id: &QItemId,
        to: &SessionRef,
        now: Timestamp,
    ) -> Result<QueueItem, Error> {
        let closed = self.must(id)?;
        let created = self.insert_new(
            NewItem {
                id: None,
                source: closed.source.clone(),
                destination: to.clone(),
                body: closed.body.clone(),
                priority: closed.priority,
                tier: closed.tier.clone(),
                tags: closed.tags.clone(),
                handed_off_from: Some(closed.id),
                last_nudge: None,
                explicit_human: to.is_human_seat(),
                fingerprint: String::new(),
            },
            now,
        );
        if let Some(row) = self.items.get_mut(id) {
            row.handed_off_to = Some(to.clone());
        }
        validate_closure("handed-off", Some("handed_off_to"), Some(to.as_str()))?;
        Ok(created)
    }

    fn must(&self, id: &QItemId) -> Result<QueueItem, Error> {
        self.items.get(id).cloned().ok_or_else(|| missing(id))
    }

    fn gate(&self, _draft: &Self, point: &'static str) -> Result<(), Error> {
        if self.crash_at == Some(point) {
            Err(Error::Crashed(point))
        } else {
            Ok(())
        }
    }

    fn swap(&mut self, draft: Self, point: &'static str) -> Result<(), Error> {
        self.gate(&draft, point)?;
        let crash = self.crash_at;
        let epoch = self.epoch.saturating_add(1);
        *self = draft;
        self.crash_at = crash;
        self.epoch = epoch;
        Ok(())
    }
}

fn ensure_claimable(
    id: &QItemId,
    item: &QueueItem,
    by: &SessionRef,
    force: Option<&str>,
) -> Result<(), Error> {
    if item.state == QueueState::InProgress {
        let allowed = force.is_some_and(|reason| !reason.trim().is_empty());
        if !allowed {
            let holder = item
                .claimed_by
                .as_ref()
                .map_or_else(|| "unknown".to_owned(), ToString::to_string);
            return Err(Error::AlreadyClaimed { by: holder });
        }
        return Ok(());
    }
    if item.destination != *by && force.is_none_or(|reason| reason.trim().is_empty()) {
        return Err(Error::Rejected {
            code: "claim_destination_mismatch",
            message: format!("qitem {id} destination is {}, not {by}", item.destination),
        });
    }
    if item.state != QueueState::Pending && item.state != QueueState::Blocked {
        return Err(not_claimable(id, item.state));
    }
    Ok(())
}

fn claim_note(force: Option<&str>) -> String {
    match force.map(str::trim).filter(|reason| !reason.is_empty()) {
        Some(reason) => format!("claimed --force: {reason}"),
        None => "claimed".to_owned(),
    }
}

fn claim_update(by: &SessionRef, note: &str) -> Update {
    Update {
        state: Some(QueueState::InProgress),
        actor: Actor::Session(by.clone()),
        note: note.to_owned(),
        closure_reason: None,
        closure_target: None,
        reopen: false,
        blocked_on: None,
    }
}

fn not_claimable(id: &QItemId, state: QueueState) -> Error {
    Error::Rejected {
        code: "qitem_not_claimable",
        message: format!(
            "qitem {id} is in state {}; only pending/blocked are claimable",
            state.as_str()
        ),
    }
}

fn missing(id: &QItemId) -> Error {
    Error::NotFound(id.to_string())
}
