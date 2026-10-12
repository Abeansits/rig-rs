//! Snapshots, checkpoints, restore, crash-cart and compaction control.
//!
//! Slice 3 adds the Codex rollout [`CodexRolloutProbe`] used to match a
//! delivery fingerprint against user items.

mod error;
mod probe;

pub use error::Error;
pub use probe::CodexRolloutProbe;
