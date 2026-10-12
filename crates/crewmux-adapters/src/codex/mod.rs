//! Codex seat: pre-flight, screen classification, launch argv, and rollout ids.
//!
//! Per-seat MCP registration is a `-c` override. Launch does not edit
//! `config.toml`.

mod adapter;
mod argv;
mod budget;
mod lock;
mod normalize;
mod preflight;
mod rollout;
mod screen;
mod submit;
mod timeutil;
mod tools;

pub use adapter::CodexAdapter;
pub use argv::{CodexLaunchPlan, codex_launch_argv};
pub use budget::LaunchBudget;
pub use lock::CodexLaunchLock;
pub use normalize::{delivery_fingerprint, normalize_for, same_delivery};
pub use preflight::{Preflight, classify_preflight, codex_preflight, codex_preflight_bin};
pub use rollout::{RolloutLocator, RolloutSet, ThreadIdError, user_turn_texts, user_turns_after};
pub use screen::{CodexScreen, ComposerState, apply_preflight, classify};
pub use tools::{RIG_TOOL_NAMES, is_rig_tool};
