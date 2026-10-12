//! Per-seat `mcp.json` and `settings.json` passed to Claude with launch flags.

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;

use serde_json::{Map, Value};

use crate::atomic::AtomicWrite;
use crate::error::ProviderError;

/// One MCP server entry written into the seat's `mcp.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerEntry {
    /// Server name.
    pub name: String,
    /// Executable.
    pub command: String,
    /// Arguments after the executable.
    pub args: Vec<String>,
}

/// Identity stamped into one Claude process's hook commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunHooks {
    /// `RIG_SEAT_KEY`.
    pub seat_key: String,
    /// `RIG_HOME`.
    pub rig_home: PathBuf,
    /// `RIG_RUN_ID`. One directory under `run/`.
    pub run_id: String,
}

/// Settings passed with `--settings`. Hooks are an arbitrary JSON object.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaudeSeatSettings {
    /// Claude permission mode, when one should be set.
    pub permission_mode: Option<String>,
    /// Hook configuration object. Unknown keys are preserved by the caller.
    pub hooks: Value,
}

/// Files under `$RIG_HOME/seats/<id>/`.
#[derive(Debug, Clone)]
pub struct ClaudeSeatFiles {
    seat_dir: PathBuf,
}

impl ClaudeSeatFiles {
    /// `seat_dir` is `$RIG_HOME/seats/<id>`.
    #[must_use]
    pub fn new(seat_dir: impl Into<PathBuf>) -> Self {
        Self {
            seat_dir: seat_dir.into(),
        }
    }

    /// Write `mcp.json` for `--mcp-config`.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Io`] when the seat directory or file cannot be written.
    pub fn write_mcp(&self, servers: &[McpServerEntry]) -> Result<PathBuf, ProviderError> {
        let path = self.seat_dir.join("mcp.json");
        let body = Self::render_mcp(servers)?;
        fs::create_dir_all(&self.seat_dir)?;
        AtomicWrite::write(&path, body.as_bytes(), Some(0o600))?;
        Ok(path)
    }

    /// Write `settings.json` for `--settings`.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Io`] when the seat directory or file cannot be written.
    pub fn write_settings(&self, settings: &ClaudeSeatSettings) -> Result<PathBuf, ProviderError> {
        let path = self.seat_dir.join("settings.json");
        let body = Self::render_settings(settings)?;
        fs::create_dir_all(&self.seat_dir)?;
        AtomicWrite::write(&path, body.as_bytes(), Some(0o600))?;
        Ok(path)
    }

    /// JSON that [`Self::write_mcp`] publishes.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Parse`] when the document cannot be encoded.
    pub fn render_mcp(servers: &[McpServerEntry]) -> Result<String, ProviderError> {
        mcp_document(servers)
    }

    /// JSON that [`Self::write_settings`] publishes.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Parse`] when the document cannot be encoded.
    pub fn render_settings(settings: &ClaudeSeatSettings) -> Result<String, ProviderError> {
        settings_document(settings)
    }

    /// Write `<seat>/run/<run-id>/settings.json` for one Claude process.
    ///
    /// The file registers `rig hook <event>` for the six compaction events and
    /// is not rewritten while that process runs. Callers pass it as `--settings`.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Io`] when the directory or file cannot be written.
    pub fn write_run_settings(
        &self,
        run: &RunHooks,
        permission_mode: Option<&str>,
    ) -> Result<PathBuf, ProviderError> {
        let dir = self.seat_dir.join("run").join(&run.run_id);
        let path = dir.join("settings.json");
        let body = render_run(run, permission_mode)?;
        fs::create_dir_all(&dir)?;
        AtomicWrite::write(&path, body.as_bytes(), Some(0o600))?;
        Ok(path)
    }

    /// `--mcp-config <p> --strict-mcp-config --settings <p>`.
    #[must_use]
    pub fn launch_flags(&self) -> Vec<OsString> {
        let mcp = self.seat_dir.join("mcp.json");
        let settings = self.seat_dir.join("settings.json");
        vec![
            OsString::from("--mcp-config"),
            mcp.into_os_string(),
            OsString::from("--strict-mcp-config"),
            OsString::from("--settings"),
            settings.into_os_string(),
        ]
    }
}

fn mcp_document(servers: &[McpServerEntry]) -> Result<String, ProviderError> {
    let mut mcp_servers = Map::new();
    for server in servers {
        let mut entry = Map::new();
        entry.insert("command".to_owned(), Value::String(server.command.clone()));
        let args = server
            .args
            .iter()
            .cloned()
            .map(Value::String)
            .collect::<Vec<_>>();
        entry.insert("args".to_owned(), Value::Array(args));
        mcp_servers.insert(server.name.clone(), Value::Object(entry));
    }
    let mut root = Map::new();
    root.insert("mcpServers".to_owned(), Value::Object(mcp_servers));
    serde_json::to_string_pretty(&Value::Object(root)).map_err(|err| ProviderError::Parse {
        file: PathBuf::from("mcp.json"),
        msg: err.to_string(),
    })
}

const HOOK_EVENTS: [&str; 6] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreCompact",
    "PostCompact",
    "Stop",
    "SessionEnd",
];

fn render_run(run: &RunHooks, permission_mode: Option<&str>) -> Result<String, ProviderError> {
    let mut hooks = Map::new();
    for event in HOOK_EVENTS {
        let command = hook_command(run, event);
        let handler = serde_json::json!({
            "type": "command",
            "command": command,
        });
        let group = serde_json::json!({ "hooks": [handler] });
        hooks.insert(event.to_owned(), Value::Array(vec![group]));
    }
    let settings = ClaudeSeatSettings {
        permission_mode: permission_mode.map(str::to_owned),
        hooks: Value::Object(hooks),
    };
    settings_document(&settings)
}

fn hook_command(run: &RunHooks, event: &str) -> String {
    format!(
        "env RIG_SEAT_KEY={} RIG_HOME={} RIG_RUN_ID={} rig hook {event}",
        shell_quote(&run.seat_key),
        shell_quote(&run.rig_home.display().to_string()),
        shell_quote(&run.run_id)
    )
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn settings_document(settings: &ClaudeSeatSettings) -> Result<String, ProviderError> {
    let mut root = Map::new();
    if let Some(mode) = &settings.permission_mode {
        let mut permissions = Map::new();
        permissions.insert("defaultMode".to_owned(), Value::String(mode.clone()));
        root.insert("permissions".to_owned(), Value::Object(permissions));
    }
    root.insert("hooks".to_owned(), settings.hooks.clone());
    serde_json::to_string_pretty(&Value::Object(root)).map_err(|err| ProviderError::Parse {
        file: PathBuf::from("settings.json"),
        msg: err.to_string(),
    })
}
