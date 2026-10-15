//! Pure answer policy. No pane I/O.

use std::path::Path;

use crate::dialog::recognise::Recognised;
use crate::dialog::table::{Answer, DialogParams, Gate};

/// Flags from the seat spec. Both default off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnswerPolicy {
    /// `RigSpec` `auto_trust`. Default false.
    pub auto_trust: bool,
    /// Reserved for ADR-0002 tool approval. Default false until D1.
    pub auto_approve_rig_tools: bool,
}

/// Paths the trust gate compares. Callers pass the seat cwd and `$HOME`.
#[derive(Clone, Copy, Debug)]
pub struct SeatContext<'a> {
    /// Seat working directory.
    pub cwd: &'a Path,
    /// Home directory. Trust is refused when the cwd is this path.
    pub home: &'a Path,
}

/// What [`plan`] decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Move the cursor to this label, then Enter once it is verified.
    Answer(&'static str),
    /// Do not send a key.
    Escalate(EscalateReason),
}

/// Why the driver stopped before Enter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EscalateReason {
    /// The table says this dialog is never answered.
    NoChoice,
    /// `auto_trust` is off, or the folder is not the seat cwd.
    Policy,
    /// The label is on the deny list (`Update now`, `No, exit`, …).
    NeverChoose,
    /// The cursor was not on the planned label after navigation.
    CursorMismatch,
    /// Enter was sent and the same dialog was still on screen.
    DialogRemained,
    /// The screen stopped matching the dialog mid-navigation.
    Unrecognised,
    /// Navigation used the step budget.
    StepLimit,
}

impl EscalateReason {
    /// Short text for tests and logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoChoice => "no automatic choice",
            Self::Policy => "policy refused the answer",
            Self::NeverChoose => "label is on the deny list",
            Self::CursorMismatch | Self::StepLimit => "cursor was not on the requested choice",
            Self::DialogRemained => "dialog still on screen after enter",
            Self::Unrecognised => "screen is not the same dialog",
        }
    }
}

const NEVER_CHOOSE: &[&str] = &[
    "Update now",
    "Always allow",
    "No, exit",
    "Run without daemon",
    "Cancel",
    "Allow",
];

/// Decide the answer. This does not look at the cursor and does not send keys.
#[must_use]
pub fn plan(recognised: &Recognised, policy: AnswerPolicy, seat: &SeatContext<'_>) -> Plan {
    match recognised.spec.answer {
        Answer::Escalate => Plan::Escalate(EscalateReason::NoChoice),
        Answer::Choose { label, gate } => {
            if never_choose(label) {
                return Plan::Escalate(EscalateReason::NeverChoose);
            }
            match gate {
                Gate::Always => Plan::Answer(label),
                Gate::AutoTrust => {
                    if policy.auto_trust && trust_ok(recognised.params.folder.as_ref(), seat) {
                        Plan::Answer(label)
                    } else {
                        Plan::Escalate(EscalateReason::Policy)
                    }
                }
                Gate::AutoApproveRigTools => {
                    if policy.auto_approve_rig_tools && approved_tool(&recognised.params) {
                        Plan::Answer(label)
                    } else {
                        Plan::Escalate(EscalateReason::Policy)
                    }
                }
            }
        }
    }
}

/// Deny list. A parenthetical suffix (`Update now (runs brew…)`) still matches.
/// `Allow for this session` does not match the exact word `Allow`.
#[must_use]
pub(super) fn never_choose(label: &str) -> bool {
    NEVER_CHOOSE.iter().any(|needle| {
        label == *needle
            || label
                .strip_prefix(needle)
                .is_some_and(|rest| rest.starts_with(" (") || rest.starts_with('('))
    })
}

fn approved_tool(params: &DialogParams) -> bool {
    let Some(tool) = params.tool.as_deref() else {
        return false;
    };
    params.server.as_deref() == Some(crewmux_provider::RIG_MCP_NAME)
        && crate::codex::is_rig_tool(tool)
}

fn trust_ok(folder: Option<&std::path::PathBuf>, seat: &SeatContext<'_>) -> bool {
    if same_dir(seat.cwd, seat.home) {
        return false;
    }
    folder
        .map(|folder| expand_tilde(folder, seat.home))
        .is_some_and(|folder| same_dir(&folder, seat.cwd))
}

/// `~` and `~/…` use the seat home. `~user` is left alone, so it cannot match.
fn expand_tilde(folder: &Path, home: &Path) -> std::path::PathBuf {
    let text = trim_dir(&folder.display().to_string());
    if text == "~" {
        return home.to_path_buf();
    }
    let Some(rest) = text.strip_prefix("~/") else {
        return std::path::PathBuf::from(text);
    };
    let mut expanded = home.to_path_buf();
    if !rest.is_empty() {
        expanded.push(rest);
    }
    expanded
}

/// Lexical directory equality. Trailing slashes and a trailing U+FFFD are ignored.
#[must_use]
pub fn same_dir(left: &Path, right: &Path) -> bool {
    trim_dir(&left.display().to_string()) == trim_dir(&right.display().to_string())
}

fn trim_dir(path: &str) -> String {
    let trimmed = path
        .trim()
        .trim_matches('\u{a0}')
        .trim()
        .trim_end_matches(['/', '\u{FFFD}'])
        .trim();
    if trimmed.is_empty() {
        "/".to_owned()
    } else {
        trimmed.to_owned()
    }
}
