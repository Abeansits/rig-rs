//! Message queue, send/ask, wake ladder, claims, and chatroom.
//!
//! The queue book and the wake ladder are pure. SQLite lives in `crewmux-store`.
//! Chat history is the `chat` module; the store implements the repository.

mod ask;
mod book;
mod closure;
mod envelope;
mod error;
mod inbox;
mod ladder;
mod machine;
mod model;
mod nudge;
mod paste_display;
mod receipt;
mod repo;
mod session;
mod state;

pub mod chat;

pub use ask::{AskCorpus, AskQuery, AskResult, ChatHit, Evidence, search};
pub use book::QueueBook;
pub use chat::resolve_sender;
pub use closure::{CLOSURE_REASONS, Closure, closure_required_at, validate_closure};
pub use envelope::{wrap_pane, wrap_reply};
pub use error::Error;
pub use inbox::{InboxEntry, InboxState, Mailboxes, OutboxRecord};
pub use ladder::{
    ESCALATION_RUNG_PREFIX, LADDER_EXHAUSTED_PREFIX, LADDER_RESUME_PREFIX, LADDER_SUSPEND_PREFIX,
    LadderView, Rung, WAKE_ATTEMPT_PREFIX, WakeAction, WakeConfig, WakeHumanRung, WakeLadder,
    dedup_escalations, escalation_reason, nudge_body, transition_note,
};
pub use machine::admit;
pub use model::{Actor, NewItem, NudgeRecord, QueueItem, Transition, Update};
pub use nudge::{leaks_body, nudge_pointer, obligation_fingerprint, unconfirmed_ids};
pub use paste_display::{PasteDisplay, claude_visible, expand_tabs, transcript_hit};
pub use receipt::{DeliveryOutcome, DeliveryReceipt};
pub use repo::QueueRepo;
pub use session::{SessionRef, is_human_seat};
pub use state::QueueState;
