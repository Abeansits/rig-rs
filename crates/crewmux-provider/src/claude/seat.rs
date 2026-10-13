//! Per-seat `mcp.json` and `settings.json` passed to Claude with launch flags.

use std::collections::BTreeMap;
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
    /// Environment for the server process. Omitted from JSON when empty.
    pub env: BTreeMap<String, String>,
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
        if !server.env.is_empty() {
            let env = server
                .env
                .iter()
                .map(|(key, value)| (key.clone(), Value::String(value.clone())))
                .collect::<Map<_, _>>();
            entry.insert("env".to_owned(), Value::Object(env));
        }
        mcp_servers.insert(server.name.clone(), Value::Object(entry));
    }
    let mut root = Map::new();
    root.insert("mcpServers".to_owned(), Value::Object(mcp_servers));
    serde_json::to_string_pretty(&Value::Object(root)).map_err(|err| ProviderError::Parse {
        file: PathBuf::from("mcp.json"),
        msg: err.to_string(),
    })
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
