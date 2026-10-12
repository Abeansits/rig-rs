//! Codex recognised dialogs. Labels are the Mac rerun captures.

use crewmux_types::DialogId;

use crate::dialog::{Answer, DialogParams, DialogSpec, Gate, TitleMatch};

/// Update, MCP tool approval, and the background-server prompt.
pub static CODEX_DIALOGS: &[DialogSpec] = &[
    DialogSpec {
        id: DialogId::CodexUpdate,
        title: TitleMatch::Contains("Update available"),
        options: &[
            "Update now (runs `brew upgrade --cask codex`)",
            "Skip",
            "Skip until next version",
        ],
        params: update_params,
        answer: Answer::Choose {
            label: "Skip",
            gate: Gate::Always,
        },
    },
    DialogSpec {
        id: DialogId::CodexMcpToolApproval,
        title: TitleMatch::Contains("MCP server to run tool"),
        options: &["Allow", "Allow for this session", "Always allow", "Cancel"],
        params: mcp_params,
        answer: Answer::Choose {
            label: "Allow for this session",
            gate: Gate::AutoApproveRigTools,
        },
    },
    DialogSpec {
        id: DialogId::CodexBackgroundServer,
        title: TitleMatch::Contains("Cannot use the background server"),
        options: &[
            "Run without daemon this time",
            "Restart with these settings (disabled)",
            "Cancel",
        ],
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

fn mcp_params(text: &str) -> DialogParams {
    for line in text.lines() {
        let trimmed = line.trim().trim_start_matches(['│', '┃', '›', '❯', ' ']);
        let Some(rest) = trimmed.strip_prefix("Allow the ") else {
            continue;
        };
        let Some((server, after)) = rest.split_once(" MCP server to run tool ") else {
            continue;
        };
        let tool = after
            .trim()
            .trim_end_matches(['?', '.'])
            .trim_matches(['"', '“', '”']);
        if !server.is_empty() && !tool.is_empty() {
            return DialogParams {
                server: Some(server.to_owned()),
                tool: Some(tool.to_owned()),
                ..DialogParams::default()
            };
        }
    }
    DialogParams::default()
}
