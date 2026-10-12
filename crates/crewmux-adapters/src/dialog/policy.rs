//! Which known dialogs may be answered without a human.

use crewmux_provider::RIG_MCP_NAME;

use crate::codex::is_rig_tool;
use crate::dialog::known::{DialogChoice, KnownDialog};

/// Opt-in flags from the seat. Both default to the slice 3 proposal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnswerPolicy {
    /// Cursor-verified Claude trust. Default off.
    pub auto_trust: bool,
    /// "Allow for this session" for server `rig` and an ADR-0002 tool name.
    pub auto_approve_rig_tools: bool,
}

impl Default for AnswerPolicy {
    fn default() -> Self {
        Self {
            auto_trust: false,
            auto_approve_rig_tools: true,
        }
    }
}

/// The choice to drive, or nothing when the dialog must escalate.
#[must_use]
pub fn planned_choice(dialog: &KnownDialog, policy: AnswerPolicy) -> Option<DialogChoice> {
    match dialog {
        KnownDialog::CodexUpdate { .. } => Some(DialogChoice::Skip),
        KnownDialog::CodexMcpToolApproval { server, tool } => approve_tool(server, tool, policy),
        KnownDialog::ClaudeTrust if policy.auto_trust => Some(DialogChoice::TrustFolder),
        KnownDialog::ClaudeTrust
        | KnownDialog::ClaudeBypass
        | KnownDialog::CodexBackgroundServer => None,
    }
}

fn approve_tool(server: &str, tool: &str, policy: AnswerPolicy) -> Option<DialogChoice> {
    if policy.auto_approve_rig_tools && server == RIG_MCP_NAME && is_rig_tool(tool) {
        Some(DialogChoice::AllowForSession)
    } else {
        None
    }
}
