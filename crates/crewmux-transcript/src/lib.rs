//! Delivery identity (ADR-0009) and provider transcript reads (ADR-0010).
//!
//! Adapters and, later, restore both depend on this crate. It does not depend
//! on adapters, so snapshot restore can call adapters without a cycle.

mod claude;
mod codex;
mod error;
mod normalize;
mod probe;
mod source;

pub use claude::{ClaudeTranscript, canonical_session_id, project_slug, transcript_path};
pub use codex::{CodexRollout, SessionMeta, read_session_meta};
pub use error::Error;
pub use normalize::{
    Fingerprint, TurnMatch, fingerprint, fingerprint_raw, match_turn, normalize_for,
    pasted_openings, unwrap_outermost,
};
pub use probe::TranscriptProbe;
pub use source::{
    Delivery, TranscriptCursor, TranscriptSource, UserTurn, user_turns_after, verify_delivery,
};
