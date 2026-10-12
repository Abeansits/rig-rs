//! Claude Code seat: launch flags, screen classification, and transcript submit.

mod adapter;
mod argv;
mod drive;
mod launch;
pub(crate) mod screen;
mod submit;

pub use adapter::ClaudeAdapter;
pub use screen::{
    ClassifiedDialog, ClaudeScreen, LoginKind, classify, classify_text, identify_dialog, readiness,
    strict_mcp_drift,
};
pub use submit::ClaudeIo;
