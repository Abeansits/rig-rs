//! Claude compaction: prompts, the pure enforcer, hook parsing, and attempts.
//!
//! Prompt text is ported from `OpenRig` v0.6.9. Completion detection is PE-018.

mod attempt;
mod clock;
mod enforcer;
mod hash;
mod hook;
mod marker;
mod prompt;
mod spool;
mod threshold;

pub use attempt::{
    AbortReason, ApplyEffect, Attempt, AttemptId, AttemptState, Book, summary_value,
};
pub use clock::{Millis, iso_utc};
pub use enforcer::{
    Enforcer, EnforcerInput, EnforcerOutcome, EnforcerSkipReason, ManualCompactionStage, PrepMode,
    PrepStatus, PreparationState, gated_submit,
};
pub use hash::{hex, sha256, sha256_hex};
pub use hook::{CompactTrigger, HookEvent, SessionSource, parse_hook_json, parse_hook_value};
pub use marker::{
    RestoreMarker, is_actionable, marker_bytes, marker_path, parse_marker, sanitize_seat_key,
};
pub use prompt::{
    RestorePrompt, build_compact_command, build_post_compact_compliance_prompt,
    build_post_compact_restore_prompt, build_post_compact_turn_boundary_prompt,
    build_pre_compact_prep_prompt, completion_marker, declared_seat_of, json_string, map_path,
};
pub use spool::{SpoolEvent, SpoolText, parse_spool_line, split_spool_text};
pub use threshold::Threshold;

/// How long a `PreCompact` may wait for both completion signals.
pub const COMPLETION_WAIT: std::time::Duration = std::time::Duration::from_secs(120);

/// Suppress a second auto `/compact` inside this window.
pub const DEDUP_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

/// After a finished restore, ignore a fresh threshold crossing for this long.
pub const POST_COMPACT_RESTORE_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(600);

/// Manual preparation budget, including the idle wait.
pub const MANUAL_PREP_WAIT: std::time::Duration = std::time::Duration::from_secs(120);

/// Automatic preparation budget. It starts after the prep prompt is delivered.
pub const AUTO_PREP_WAIT: std::time::Duration = std::time::Duration::from_mins(25);

/// Lower bound between hook completion and the restore prompt.
///
/// Elapsing this wait does not send the restore. The seat must also be `Ready`
/// and both completion signals must have arrived.
pub const POST_COMPACT_SEND_WAIT: std::time::Duration = std::time::Duration::from_secs(10);
