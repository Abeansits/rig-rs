//! Dialog identifiers and the Claude composer flag.

use serde::{Deserialize, Serialize};

/// A provider dialog the driver can recognise.
///
/// Parameters (folder, server, tool, versions) stay in the adapter tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DialogId {
    /// "Accessing workspace" / "Do you trust the files in this folder?".
    ClaudeTrust,
    /// Bypass-permissions consent. Never auto-answered.
    ClaudeBypassConsent,
    /// "New MCP server found". Never auto-answered.
    ClaudeNewMcpServer,
    /// "Try the new fullscreen renderer?".
    ClaudeFullscreenRenderer,
    /// Codex "Update available". The safe answer is Skip, never Update now.
    CodexUpdate,
    /// Codex MCP tool approval. Auto-approve is still pending (D1).
    CodexMcpToolApproval,
    /// Codex background server prompt. Never auto-answered.
    CodexBackgroundServer,
}

/// Positive composer match. Ready screens carry this; menus do not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposerState {
    /// The input border (`╭` / `╰` or a `─` rule) was on screen.
    pub bordered: bool,
}
