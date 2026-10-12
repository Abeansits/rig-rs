//! Dialogs the driver is allowed to name.

/// A provider dialog identified by its title and its full option list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KnownDialog {
    /// Codex "Update available". The safe answer is Skip, never Update now.
    CodexUpdate {
        /// Installed version.
        from: String,
        /// Offered version.
        to: String,
    },
    /// "Allow the … MCP server to run tool …".
    CodexMcpToolApproval {
        /// Server name from the title.
        server: String,
        /// Tool name from the title.
        tool: String,
    },
    /// "Cannot use the background server". Never answered.
    CodexBackgroundServer,
    /// Claude "Accessing workspace". Answered only when the folder is the seat cwd.
    ClaudeTrust {
        /// Absolute folder from the dialog. Missing means fail closed.
        folder: Option<std::path::PathBuf>,
    },
    /// Claude bypass consent. Always escalated.
    ClaudeBypass,
}

/// An answer the driver can move the cursor to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogChoice {
    /// Codex update option "Skip".
    Skip,
    /// Codex MCP option "Allow for this session".
    AllowForSession,
    /// Claude trust option "Yes, I trust this folder".
    TrustFolder,
}

impl DialogChoice {
    /// Exact option label. "Skip" does not match "Skip until next version".
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Skip => "Skip",
            Self::AllowForSession => "Allow for this session",
            Self::TrustFolder => "Yes, I trust this folder",
        }
    }
}
