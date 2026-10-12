//! Dialogs and the Claude composer, shared by the screen classifier.

use serde::{Deserialize, Serialize};

/// A provider dialog identified by its title and its full option list.
///
/// Codex variants besides the update prompt are slice 3. The update prompt is
/// here because a blind Enter on it would run a package upgrade.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnownDialog {
    /// "Accessing workspace" / "Do you trust the files in this folder?".
    ClaudeTrust,
    /// Bypass-permissions consent. Never auto-answered.
    ClaudeBypassConsent,
    /// "New MCP server found". Never auto-answered.
    ClaudeNewMcpServer {
        /// Server name taken from the title line.
        name: String,
    },
    /// "Try the new fullscreen renderer?".
    ClaudeFullscreenRenderer,
    /// "Update available". The safe answer is Skip, never Update now.
    CodexUpdate,
}

/// Positive composer match. Ready screens carry this; menus do not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposerState {
    /// The input border (`╭` / `╰` or a `─` rule) was on screen.
    pub bordered: bool,
}
