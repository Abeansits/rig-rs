//! Codex recognised dialogs. The table is enough for the update prompt; the
//! Codex adapter itself is slice 3.

use crewmux_types::DialogId;

use crate::dialog::{Answer, DialogParams, DialogSpec, Gate, TitleMatch};

/// Update, MCP tool approval, and the background-server prompt.
pub static CODEX_DIALOGS: &[DialogSpec] = &[
    DialogSpec {
        id: DialogId::CodexUpdate,
        title: TitleMatch::Contains("Update available"),
        options: &["Update now (runs brew upgrade --cask codex)", "Skip"],
        params: update_params,
        answer: Answer::Choose {
            label: "Skip",
            gate: Gate::Always,
        },
    },
    DialogSpec {
        id: DialogId::CodexMcpToolApproval,
        title: TitleMatch::Contains("MCP tool"),
        options: &["Allow for this session", "Always allow", "Deny"],
        params: empty_params,
        answer: Answer::Choose {
            label: "Allow for this session",
            gate: Gate::AutoApproveRigTools,
        },
    },
    DialogSpec {
        id: DialogId::CodexBackgroundServer,
        title: TitleMatch::Contains("background server"),
        options: &["Run without daemon", "Allow"],
        params: empty_params,
        answer: Answer::Escalate,
    },
];

fn empty_params(_text: &str) -> DialogParams {
    DialogParams::default()
}

fn update_params(text: &str) -> DialogParams {
    let mut params = DialogParams::default();
    for line in text.lines() {
        let Some((from, to)) = line.trim().split_once('→') else {
            continue;
        };
        let from = from.split_whitespace().last().unwrap_or("").trim();
        let to = to.split_whitespace().next().unwrap_or("").trim();
        if !from.is_empty() && !to.is_empty() {
            params.from = Some(from.to_owned());
            params.to = Some(to.to_owned());
            break;
        }
    }
    params
}
