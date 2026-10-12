//! Codex launch argv. MCP registration is `-c` overrides, not a config edit.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crewmux_provider::{RIG_MCP_NAME, rig_mcp_command};
use crewmux_types::{CodexApproval, ThreadId};

/// Inputs for [`codex_launch_argv`].
#[derive(Clone, Debug)]
pub struct CodexLaunchPlan {
    /// `codex` binary, or a test stub.
    pub bin: PathBuf,
    /// Approval flag. `never` is rewritten to `on-request`.
    pub approval: CodexApproval,
    /// `resume <thread-id>` when relaunching.
    pub resume: Option<ThreadId>,
    /// Environment entries published as `mcp_servers.rig.env.*`.
    pub mcp_env: BTreeMap<String, String>,
}

/// Argv for one seat. Does not mention `config.toml`.
#[must_use]
pub fn codex_launch_argv(plan: &CodexLaunchPlan) -> Vec<String> {
    let mut argv = vec![
        plan.bin.display().to_string(),
        "-a".to_owned(),
        plan.approval.launch_arg().to_owned(),
    ];
    push_mcp(&mut argv, &plan.mcp_env);
    if let Some(id) = &plan.resume {
        argv.push("resume".to_owned());
        argv.push(id.as_str().to_owned());
    }
    argv
}

fn push_mcp(argv: &mut Vec<String>, env: &BTreeMap<String, String>) {
    let command = rig_mcp_command();
    let Some(bin) = command.first() else {
        return;
    };
    let extra = command.get(1..).unwrap_or(&[]);
    push_override(
        argv,
        &format!("mcp_servers.{RIG_MCP_NAME}.command={}", toml_basic(bin)),
    );
    push_override(
        argv,
        &format!("mcp_servers.{RIG_MCP_NAME}.args={}", toml_array(extra)),
    );
    for (key, value) in env {
        push_override(
            argv,
            &format!("mcp_servers.{RIG_MCP_NAME}.env.{key}={}", toml_basic(value)),
        );
    }
}

fn push_override(argv: &mut Vec<String>, value: &str) {
    argv.push("-c".to_owned());
    argv.push(value.to_owned());
}

fn toml_basic(text: &str) -> String {
    if text
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '/' | '.'))
    {
        return text.to_owned();
    }
    let mut out = String::from("\"");
    for ch in text.chars() {
        if ch == '\\' || ch == '"' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

fn toml_array(items: &[String]) -> String {
    let mut out = String::from("[");
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('"');
        for ch in item.chars() {
            if ch == '\\' || ch == '"' {
                out.push('\\');
            }
            out.push(ch);
        }
        out.push('"');
    }
    out.push(']');
    out
}
