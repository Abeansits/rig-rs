//! Claude recognised dialogs. Labels are taken from the 2.1.286 captures.

use std::path::PathBuf;

use crewmux_types::DialogId;

use crate::dialog::{Answer, DialogParams, DialogSpec, Gate, TitleMatch};

/// Trust, bypass, a new MCP server, and the fullscreen renderer.
pub(super) static CLAUDE_DIALOGS: &[DialogSpec] = &[
    DialogSpec {
        id: DialogId::ClaudeTrust,
        title: TitleMatch::Contains("Accessing workspace"),
        options: &["No, exit", "Yes, I trust this folder"],
        params: trust_params,
        answer: Answer::Choose {
            label: "Yes, I trust this folder",
            gate: Gate::AutoTrust,
        },
    },
    DialogSpec {
        id: DialogId::ClaudeBypassConsent,
        title: TitleMatch::Contains("WARNING: Claude Code running in Bypass Permissions mode"),
        options: &["No, exit", "Yes, I accept"],
        params: empty_params,
        answer: Answer::Escalate,
    },
    DialogSpec {
        id: DialogId::ClaudeNewMcpServer,
        title: TitleMatch::Contains("New MCP server found in this project"),
        options: &[
            "Use this MCP server",
            "Use this and all future MCP servers in this project",
            "Continue without using this MCP server",
        ],
        params: mcp_params,
        answer: Answer::Escalate,
    },
    DialogSpec {
        id: DialogId::ClaudeFullscreenRenderer,
        title: TitleMatch::Contains("Try the new fullscreen renderer?"),
        options: &["Yes, try it", "Not now"],
        params: empty_params,
        answer: Answer::Choose {
            label: "Not now",
            gate: Gate::Always,
        },
    },
];

fn empty_params(_text: &str) -> DialogParams {
    DialogParams::default()
}

fn trust_params(text: &str) -> DialogParams {
    DialogParams {
        folder: trust_folder(text),
        ..DialogParams::default()
    }
}

fn mcp_params(text: &str) -> DialogParams {
    DialogParams {
        server: mcp_server(text),
        ..DialogParams::default()
    }
}

/// Folder path printed under "Accessing workspace".
#[must_use]
pub(crate) fn trust_folder(text: &str) -> Option<PathBuf> {
    let lines: Vec<&str> = text.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("Accessing workspace") else {
            continue;
        };
        let rest = rest.trim().trim_start_matches(':').trim();
        if rest.starts_with('/') {
            return Some(PathBuf::from(rest));
        }
        let later = lines.get(index.saturating_add(1)..).unwrap_or(&[]);
        for next in later {
            let next = next.trim();
            if next.is_empty() {
                continue;
            }
            if next.starts_with('/') {
                return Some(PathBuf::from(next));
            }
            break;
        }
    }
    None
}

fn mcp_server(text: &str) -> Option<String> {
    for line in text.lines() {
        let Some(rest) = line
            .trim()
            .strip_prefix("New MCP server found in this project:")
        else {
            continue;
        };
        let name = rest.trim().trim_end_matches('.');
        if !name.is_empty() {
            return Some(name.to_owned());
        }
    }
    None
}
