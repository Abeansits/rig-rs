//! Which known dialogs may be answered without a human.

use std::path::{Path, PathBuf};

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

/// Seat paths a trust dialog must match. `$HOME` is never trusted.
#[derive(Clone, Copy, Debug)]
pub struct TrustSeat<'a> {
    /// Seat working directory.
    pub cwd: &'a Path,
    /// The seat's `$HOME`.
    pub home: &'a Path,
}

/// The choice to drive, or nothing when the dialog must escalate.
///
/// Claude trust is answered only when `seat` is present, the dialog names a
/// folder, that folder canonicalises to `seat.cwd`, and it is not `$HOME`.
#[must_use]
pub fn planned_choice(
    dialog: &KnownDialog,
    policy: AnswerPolicy,
    seat: Option<TrustSeat<'_>>,
) -> Option<DialogChoice> {
    match dialog {
        KnownDialog::CodexUpdate { .. } => Some(DialogChoice::Skip),
        KnownDialog::CodexMcpToolApproval { server, tool } => approve_tool(server, tool, policy),
        KnownDialog::ClaudeTrust { folder } if policy.auto_trust => {
            trust_choice(folder.as_deref(), seat)
        }
        KnownDialog::ClaudeTrust { .. }
        | KnownDialog::ClaudeBypass
        | KnownDialog::CodexBackgroundServer => None,
    }
}

/// Absolute path on an "Accessing workspace" title line, or the next line.
#[must_use]
pub fn trust_folder(text: &str) -> Option<PathBuf> {
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let rest = line
            .trim()
            .trim_start_matches(|ch: char| !ch.is_ascii_alphanumeric());
        if !rest.starts_with("Accessing workspace") {
            continue;
        }
        if let Some(path) = absolute_path(rest) {
            return Some(path);
        }
        let next = lines.find(|line| !line.trim().is_empty())?;
        return absolute_path(next.trim());
    }
    None
}

fn trust_choice(folder: Option<&Path>, seat: Option<TrustSeat<'_>>) -> Option<DialogChoice> {
    let folder = folder?;
    let seat = seat?;
    if folder_matches(folder, seat.cwd, seat.home) {
        Some(DialogChoice::TrustFolder)
    } else {
        None
    }
}

fn folder_matches(folder: &Path, cwd: &Path, home: &Path) -> bool {
    let Ok(folder) = std::fs::canonicalize(folder) else {
        return false;
    };
    let Ok(cwd) = std::fs::canonicalize(cwd) else {
        return false;
    };
    let Ok(home) = std::fs::canonicalize(home) else {
        return false;
    };
    folder == cwd && folder != home
}

fn absolute_path(text: &str) -> Option<PathBuf> {
    let token = text
        .split_whitespace()
        .find(|token| token.starts_with('/'))?;
    let token = token.trim_end_matches(['?', '.', ',']);
    if token.starts_with('/') && token.len() > 1 {
        Some(PathBuf::from(token))
    } else {
        None
    }
}

fn approve_tool(server: &str, tool: &str, policy: AnswerPolicy) -> Option<DialogChoice> {
    if policy.auto_approve_rig_tools && server == RIG_MCP_NAME && is_rig_tool(tool) {
        Some(DialogChoice::AllowForSession)
    } else {
        None
    }
}
