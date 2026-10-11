//! Format-preserving, atomic writers for provider and tmux configuration files.
//!
//! Claude per-seat files, the guarded trust pre-accept, Codex `config.toml`,
//! the per-seat `OpenCode` config, the tmux managed block, and the private Herdr
//! launch config all go through [`atomic::AtomicWrite`].

mod atomic;
mod auth;
mod backup;
mod claude;
mod codex;
mod codex_block;
mod drift;
mod error;
mod guidance;
mod herdr;
mod json_edit;
mod opencode;
mod tmux_conf;

pub use atomic::{AtomicWrite, StagedWrite, stage};
pub use auth::{CodexProviderAuth, select_codex_provider_auth};
pub use backup::Backups;
pub use claude::{
    ClaudeSeatFiles, ClaudeSeatSettings, ClaudeTrust, FileStamp, McpServerEntry, ProcessProbe,
    TrustOutcome, user_config_basename,
};
pub use codex::{
    Change, CodexConfig, CodexHealth, RIG_MCP_NAME, TrustLevel, classify_config, rig_mcp_command,
};
pub use drift::DriftReport;
pub use error::ProviderError;
pub use guidance::{MemoryFs, merge_managed_block};
pub use herdr::HerdrLaunchConfig;
pub use opencode::{OpenCodePermission, OpenCodeSeatConfig};
pub use tmux_conf::{
    ManagedBlock, TMUX_BLOCK_BODY, global_window_size_manual, managed_block_current,
    render_tmux_block,
};
