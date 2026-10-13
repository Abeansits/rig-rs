//! Claude argv. The session id is minted before the process is started.

use std::path::Path;

/// `--session-id` or `--resume`, then the per-seat MCP and settings flags.
#[must_use]
pub(crate) fn claude_argv(
    binary: &str,
    session_id: &str,
    resume: bool,
    mcp: &Path,
    settings: &Path,
    model: Option<&str>,
) -> Vec<String> {
    let mut args = vec![binary.to_owned()];
    if resume {
        args.push("--resume".to_owned());
    } else {
        args.push("--session-id".to_owned());
    }
    args.push(session_id.to_owned());
    if let Some(model) = model {
        args.push("--model".to_owned());
        args.push(model.to_owned());
    }
    args.push("--mcp-config".to_owned());
    args.push(mcp.display().to_string());
    args.push("--strict-mcp-config".to_owned());
    args.push("--allowedTools".to_owned());
    args.push(crewmux_provider::claude_allowed_tools().join(","));
    args.push("--settings".to_owned());
    args.push(settings.display().to_string());
    args
}

/// New random Claude session id.
#[must_use]
pub(crate) fn mint_session_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
