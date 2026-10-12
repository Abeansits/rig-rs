//! Recognised-dialog tables. Each adapter supplies one.

use std::path::PathBuf;

use crewmux_types::DialogId;

/// How a title line is recognised.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleMatch {
    /// One line contains this text.
    Contains(&'static str),
    /// Every phrase appears on some line.
    AllOf(&'static [&'static str]),
}

/// What the driver should do once the dialog is recognised.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Stop. Do not move the cursor and do not send Enter.
    Escalate,
    /// Move to `label` when `gate` allows it.
    Choose {
        /// Exact label after the number prefix is stripped.
        label: &'static str,
        /// Extra condition beyond recognition.
        gate: Gate,
    },
}

/// When a recognised choice may be submitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// The choice is always safe (fullscreen "Not now", Codex "Skip").
    Always,
    /// `auto_trust` and the folder is the seat cwd, which is not `$HOME`.
    AutoTrust,
    /// Policy flag and the ADR-0002 tool allow-list. The list is empty until D1.
    AutoApproveRigTools,
}

/// Facts pulled out of a dialog. Extraction never decides the answer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DialogParams {
    /// Trust-dialog folder, when the title printed one.
    pub folder: Option<PathBuf>,
    /// MCP server name.
    pub server: Option<String>,
    /// MCP tool name.
    pub tool: Option<String>,
    /// Update-dialog version the user is on.
    pub from: Option<String>,
    /// Update-dialog version on offer.
    pub to: Option<String>,
}

/// One row of a provider's recognised-dialog table.
pub struct DialogSpec {
    /// Stable id.
    pub id: DialogId,
    /// Title that must be on screen.
    pub title: TitleMatch,
    /// Full option list, number prefixes stripped, in screen order.
    pub options: &'static [&'static str],
    /// Pull folder / server / versions out of the capture.
    pub params: fn(&str) -> DialogParams,
    /// Planned answer. Policy can still refuse it.
    pub answer: Answer,
}

/// A provider's recognised dialogs.
pub trait DialogTable {
    /// Rows in match order.
    fn dialogs(&self) -> &'static [DialogSpec];
}
