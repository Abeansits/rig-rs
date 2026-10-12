//! Claude Code seat: launch flags, screen classification, and transcript submit.

mod adapter;
mod argv;
mod composer;
mod dialogs;
mod drive;
mod launch;
pub(crate) mod screen;
mod staged;
mod submit;

pub use adapter::ClaudeAdapter;
pub use screen::{
    ClassifiedDialog, ClaudeScreen, LoginKind, classify, classify_text, readiness, strict_mcp_drift,
};
pub use submit::ClaudeIo;
