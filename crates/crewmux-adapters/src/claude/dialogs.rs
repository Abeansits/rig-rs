//! Claude recognised dialogs. Labels are taken from the 2.1.286 captures.

use std::path::PathBuf;

use crewmux_types::DialogId;

use crate::dialog::{Answer, DialogParams, DialogSpec, Gate, TitleMatch};

/// Trust, bypass, a new MCP server, and the fullscreen renderer.
pub(crate) static CLAUDE_DIALOGS: &[DialogSpec] = &[
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
///
/// Claude Code 2.1.287 draws `xn(cwd)` in an Ink row (`paddingX: 1`) under a
/// top border. `xn` appends U+FFFD when it rewrites the string. A visible
/// capture keeps that row's trailing padding and keeps an Ink hard-wrap as
/// extra rows (`capture-pane` without `-J`). The same binary's path helper
/// prints `~/…` for a directory under `$HOME`.
#[must_use]
pub(crate) fn trust_folder(text: &str) -> Option<PathBuf> {
    let lines: Vec<&str> = text.lines().collect();
    let index = lines
        .iter()
        .position(|line| line.contains("Accessing workspace"))?;
    let mut parts = Vec::new();
    if let Some(line) = lines.get(index)
        && let Some(rest) = after_title(line)
        && let Some(part) = path_piece(rest)
        && is_path_start(&part)
    {
        parts.push(part);
    }
    let later = lines.get(index.saturating_add(1)..).unwrap_or(&[]);
    for line in later {
        if line.trim().is_empty() {
            if parts.is_empty() {
                continue;
            }
            break;
        }
        let Some(part) = path_piece(line) else {
            break;
        };
        if parts.is_empty() {
            if !is_path_start(&part) {
                break;
            }
        } else if !is_path_start(&part) && !is_wrap(&part) {
            break;
        }
        parts.push(part);
    }
    if parts.is_empty() {
        None
    } else {
        Some(PathBuf::from(parts.concat()))
    }
}

fn after_title(line: &str) -> Option<&str> {
    let rest = line.trim().strip_prefix("Accessing workspace")?;
    let rest = rest.trim().trim_start_matches(':').trim();
    if rest.is_empty() { None } else { Some(rest) }
}

/// One physical row of the folder, without pane padding, box drawing, or U+FFFD.
fn path_piece(line: &str) -> Option<String> {
    let text = line.trim_matches(|ch: char| ch.is_whitespace() || ch == '│' || ch == '\u{FFFD}');
    if text.is_empty() || text.chars().any(char::is_whitespace) {
        None
    } else {
        Some(text.to_owned())
    }
}

fn is_path_start(part: &str) -> bool {
    part.starts_with('/') || part == "~" || part.starts_with("~/")
}

/// A hard-wrapped tail: path characters, not a new absolute or `~/` row.
fn is_wrap(part: &str) -> bool {
    !part.is_empty()
        && part
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | '+' | '@'))
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::trust_folder;

    #[test]
    fn joins_a_wrapped_padded_folder_and_drops_the_rewrite_mark() {
        let screen = "Accessing workspace:\n\n /Users/rig/work/\u{FFFD}   \n src/app\n\n Quick safety check\n";
        let folder = trust_folder(screen).expect("folder");
        assert_eq!(folder.as_path(), Path::new("/Users/rig/work/src/app"));
    }

    #[test]
    fn keeps_a_tilde_folder_for_the_policy_to_expand() {
        let screen = "Accessing workspace:\n\n ~/proj\n\n Quick safety check\n";
        let folder = trust_folder(screen).expect("folder");
        assert_eq!(folder.as_path(), Path::new("~/proj"));
    }

    #[test]
    fn a_different_folder_stays_different() {
        let screen = "Accessing workspace:\n\n /tmp/other\n\n Quick safety check\n";
        let folder = trust_folder(screen).expect("folder");
        assert_ne!(folder.as_path(), Path::new("/tmp/mine"));
    }
}
