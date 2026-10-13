//! Codex launch argv. MCP registration is `-c` overrides, not a config edit.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crewmux_provider::{SeatEnv, codex_mcp_overrides};
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
    /// Absolute `rig` binary passed as `mcp_servers.rig.command`.
    pub rig_bin: PathBuf,
}

/// Argv for one seat. Does not mention `config.toml`.
#[must_use]
pub fn codex_launch_argv(plan: &CodexLaunchPlan) -> Vec<String> {
    let mut argv = vec![
        plan.bin.display().to_string(),
        "-a".to_owned(),
        plan.approval.launch_arg().to_owned(),
    ];
    argv.extend(codex_mcp_overrides(&plan.rig_bin, &seat_env(&plan.mcp_env)));
    if let Some(id) = &plan.resume {
        argv.push("resume".to_owned());
        argv.push(id.as_str().to_owned());
    }
    argv
}

fn seat_env(env: &BTreeMap<String, String>) -> SeatEnv {
    SeatEnv {
        seat_id: env
            .get(crewmux_types::SEAT_ID_ENV)
            .cloned()
            .unwrap_or_default(),
        seat_token: env
            .get(crewmux_types::SEAT_TOKEN_ENV)
            .cloned()
            .unwrap_or_default(),
        rig_home: env.get("RIG_HOME").cloned().unwrap_or_default(),
    }
}
