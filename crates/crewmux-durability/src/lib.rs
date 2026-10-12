//! Snapshots, checkpoints, restore, crash-cart and compaction control.
//!
//! Slice 2 adds the Claude transcript probe and the ADR-0009 normaliser.
//! Queue restore and snapshot orchestration stay in later slices.

mod check;
mod error;
mod normalize;
mod probe;

pub use check::ClaudeDelivery;
pub use error::Error;
pub use normalize::{
    DeliveryMatch, DeliveryRuntime, fingerprint, fingerprint_raw, match_for, normalize_for,
    pasted_openings, unwrap_outermost,
};
pub use probe::{UserTurn, project_slug, transcript_len, transcript_path, user_turns_after};
