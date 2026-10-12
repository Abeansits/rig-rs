//! Queue rows, transitions, and the fields a write may change.

use crewmux_core::Timestamp;
use crewmux_types::QItemId;
use serde::{Deserialize, Serialize};

use crate::closure::Closure;
use crate::session::SessionRef;
use crate::state::QueueState;

/// Who wrote a transition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Actor {
    /// A seat session.
    Session(SessionRef),
    /// The wake ladder (`wake-ladder@system`).
    System,
}

impl Actor {
    /// Transition-log actor token.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Session(session) => session.as_str(),
            Self::System => "wake-ladder@system",
        }
    }
}

/// One recorded nudge. The body is the pointer, never the item body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NudgeRecord {
    /// When the ladder or create path nudged.
    pub at: Timestamp,
    /// Outcome token (`failed:…`, `delivered-ack-pending`, `verified`).
    pub result: String,
    /// Pointer text that was sent.
    pub text: String,
}

/// A queue row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueItem {
    /// Id.
    pub id: QItemId,
    /// Author session.
    pub source: SessionRef,
    /// Owner session.
    pub destination: SessionRef,
    /// Current state.
    pub state: QueueState,
    /// Sort key. Higher is sooner.
    pub priority: i64,
    /// SLA tier (`fast`, `routine`, `deep`, `critical`).
    pub tier: Option<String>,
    /// Free-form tags.
    pub tags: Vec<String>,
    /// Park target.
    pub blocked_on: Option<SessionRef>,
    /// Successor session after a handoff.
    pub handed_off_to: Option<SessionRef>,
    /// Predecessor item id.
    pub handed_off_from: Option<QItemId>,
    /// Optional expiry.
    pub expires_at: Option<Timestamp>,
    /// Hot-potato closure, when the state admits one.
    pub closure: Option<Closure>,
    /// SLA deadline computed at claim. Independent of [`Closure`].
    pub closure_required_at: Option<Timestamp>,
    /// When the current claim started.
    pub claimed_at: Option<Timestamp>,
    /// Claimant. Absent when unclaimed.
    pub claimed_by: Option<SessionRef>,
    /// Latest nudge. Absent until the first attempt.
    pub last_nudge: Option<NudgeRecord>,
    /// Obligation body. The wake ladder never copies this into a nudge.
    pub body: String,
    /// Delivery fingerprint of the obligation.
    pub fingerprint: String,
    /// The author explicitly addressed a human seat.
    pub explicit_human: bool,
    /// Created.
    pub created_at: Timestamp,
    /// Last mutation.
    pub updated_at: Timestamp,
}

/// One append-only transition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition {
    /// Monotonic id inside the book.
    pub id: u64,
    /// Item.
    pub qitem: QItemId,
    /// State before the write. Equal to `to` for a note append.
    pub from: QueueState,
    /// State after the write.
    pub to: QueueState,
    /// Note. Ladder markers use the TypeScript prefixes.
    pub note: String,
    /// When.
    pub at: Timestamp,
    /// Writer.
    pub actor: Actor,
}

/// Fields for [`crate::QueueBook::create`].
#[derive(Clone, Debug)]
pub struct NewItem {
    /// When set, this id is used. Otherwise a `qitem-<ulid>` is minted.
    pub id: Option<QItemId>,
    /// Author.
    pub source: SessionRef,
    /// Owner.
    pub destination: SessionRef,
    /// Body.
    pub body: String,
    /// Priority. Default 0.
    pub priority: i64,
    /// Tier.
    pub tier: Option<String>,
    /// Tags.
    pub tags: Vec<String>,
    /// Predecessor, for a baton seeded without `handoff`.
    pub handed_off_from: Option<QItemId>,
    /// Seed a failed or unconfirmed nudge.
    pub last_nudge: Option<NudgeRecord>,
    /// Explicit human address.
    pub explicit_human: bool,
    /// Fingerprint. Empty becomes a hash of the body.
    pub fingerprint: String,
}

/// A state write.
#[derive(Clone, Debug)]
pub struct Update {
    /// Next state. `None` appends a note without changing state.
    pub state: Option<QueueState>,
    /// Actor.
    pub actor: Actor,
    /// Note.
    pub note: String,
    /// Closure reason.
    pub closure_reason: Option<String>,
    /// Closure target.
    pub closure_target: Option<String>,
    /// Deliberate reopen of a guarded terminal row.
    pub reopen: bool,
    /// Park target. Only admitted for `blocked`.
    pub blocked_on: Option<SessionRef>,
}
