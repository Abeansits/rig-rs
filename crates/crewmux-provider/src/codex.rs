//! Format-preserving edits of Codex `config.toml` via `toml_edit`.
//!
//! New tables are explicit (`[mcp_servers.<name>]`, `[projects."<path>"]`).
//! The original bytes are kept and extended; removal writes those bytes back.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Table, Value};

use crate::atomic::AtomicWrite;
use crate::backup::Backups;
use crate::error::ProviderError;

/// Server name setup writes into `[mcp_servers]`.
pub const RIG_MCP_NAME: &str = "rig";

/// `rig mcp`, the command setup records for [`RIG_MCP_NAME`].
#[must_use]
pub fn rig_mcp_command() -> Vec<String> {
    vec!["rig".to_owned(), "mcp".to_owned()]
}

/// How a Codex config relates to the rig server entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexHealth {
    /// The file is missing, or the rig entry matches.
    Current,
    /// The rig entry is present with a different command.
    Drift,
}

/// Classify `text` without writing it.
///
/// # Errors
///
/// Returns [`ProviderError::Parse`] when `text` is not TOML.
pub fn classify_config(text: &str) -> Result<CodexHealth, ProviderError> {
    let path = Path::new("config.toml");
    let doc = parse_doc(path, text)?;
    let cmd = rig_mcp_command();
    let Some(command) = cmd.first() else {
        return Ok(CodexHealth::Current);
    };
    let args = cmd.get(1..).unwrap_or(&[]);
    if mcp_matches(&doc, RIG_MCP_NAME, command, args) || !mcp_exists(&doc, RIG_MCP_NAME) {
        Ok(CodexHealth::Current)
    } else {
        Ok(CodexHealth::Drift)
    }
}

/// Whether an edit changed the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// The document already had the requested state.
    Unchanged,
    /// A table was appended.
    Added,
    /// Our tables were removed and the loaded bytes restored.
    Removed,
}

/// How much Codex trusts a project directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustLevel {
    /// Codex may run in the project without a trust prompt.
    Trusted,
    /// Codex should keep the project untrusted.
    Untrusted,
}

impl TrustLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::Untrusted => "untrusted",
        }
    }
}

/// A Codex `config.toml` kept as a `toml_edit` document plus the loaded bytes.
#[derive(Debug)]
pub struct CodexConfig {
    doc: DocumentMut,
    path: PathBuf,
    original: String,
    rendered: String,
    owned_mcp: BTreeSet<String>,
    owned_projects: BTreeSet<String>,
    restore_original: bool,
}

impl CodexConfig {
    /// Load `path`. A missing file is an empty document. Invalid TOML is refused
    /// and nothing is written.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Parse`] when the file is not TOML, and
    /// [`ProviderError::Io`] when it cannot be read.
    pub fn load(path: &Path) -> Result<Self, ProviderError> {
        let original = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error.into()),
        };
        let doc = parse_doc(path, &original)?;
        Ok(Self {
            doc,
            path: path.to_path_buf(),
            rendered: original.clone(),
            original,
            owned_mcp: BTreeSet::new(),
            owned_projects: BTreeSet::new(),
            restore_original: false,
        })
    }

    /// Append `[mcp_servers.<name>]` when that server is not already present.
    ///
    /// `cmd` is the executable followed by its arguments.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Parse`] when `cmd` is empty or `name` is empty.
    pub fn ensure_mcp_server(
        &mut self,
        name: &str,
        cmd: &[String],
    ) -> Result<Change, ProviderError> {
        if name.is_empty() || cmd.is_empty() {
            return Err(ProviderError::Parse {
                file: self.path.clone(),
                msg: "mcp server needs a name and a command".to_owned(),
            });
        }
        let Some(command) = cmd.first() else {
            return Err(ProviderError::Parse {
                file: self.path.clone(),
                msg: "mcp server needs a command".to_owned(),
            });
        };
        let args = cmd.get(1..).unwrap_or(&[]);
        if mcp_matches(&self.doc, name, command, args) {
            return Ok(Change::Unchanged);
        }
        if mcp_exists(&self.doc, name) {
            return Ok(Change::Unchanged);
        }
        let addition = explicit_table(
            "mcp_servers",
            name,
            &[
                ("command", Item::Value(Value::from(command.as_str()))),
                ("args", string_array(args)),
            ],
        );
        self.append_addition(&addition)?;
        self.owned_mcp.insert(name.to_owned());
        Ok(Change::Added)
    }

    /// Append `[projects."<path>"]` with `trust_level` when it is not already set.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Parse`] when the addition cannot be encoded.
    pub fn ensure_project_trust(
        &mut self,
        project: &Path,
        level: TrustLevel,
    ) -> Result<Change, ProviderError> {
        let key = project.to_string_lossy();
        if project_matches(&self.doc, key.as_ref(), level.as_str()) {
            return Ok(Change::Unchanged);
        }
        if project_exists(&self.doc, key.as_ref()) {
            return Ok(Change::Unchanged);
        }
        let addition = explicit_table(
            "projects",
            key.as_ref(),
            &[("trust_level", Item::Value(Value::from(level.as_str())))],
        );
        self.append_addition(&addition)?;
        self.owned_projects.insert(key.into_owned());
        Ok(Change::Added)
    }

    /// Drop every table this value added and restore the bytes from [`Self::load`].
    #[must_use]
    pub fn remove_ours(&mut self) -> Change {
        if self.owned_mcp.is_empty() && self.owned_projects.is_empty() {
            return Change::Unchanged;
        }
        self.owned_mcp.clear();
        self.owned_projects.clear();
        self.rendered.clone_from(&self.original);
        self.restore_original = true;
        if let Ok(doc) = parse_doc(&self.path, &self.original) {
            self.doc = doc;
        }
        Change::Removed
    }

    /// Bytes that [`Self::save`] will publish.
    #[must_use]
    pub fn rendered(&self) -> &str {
        &self.rendered
    }

    /// Publish the document with an atomic replace. The first save of an existing
    /// file stores one backup under `backups`.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Io`] when the backup or the replace fails.
    pub fn save(&self, backups: &Backups) -> Result<(), ProviderError> {
        backups.capture_once(&self.path)?;
        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        AtomicWrite::write(&self.path, self.rendered.as_bytes(), None)?;
        Ok(())
    }
}

fn parse_doc(path: &Path, text: &str) -> Result<DocumentMut, ProviderError> {
    text.parse::<DocumentMut>()
        .map_err(|err| ProviderError::Parse {
            file: path.to_path_buf(),
            msg: err.to_string(),
        })
}

fn append_owned(config: &mut CodexConfig, addition: &str) -> Result<(), ProviderError> {
    if !config.rendered.is_empty() && !config.rendered.ends_with('\n') {
        config.rendered.push('\n');
    }
    if !config.rendered.is_empty() && !config.rendered.ends_with("\n\n") {
        config.rendered.push('\n');
    }
    config.rendered.push_str(addition);
    if !config.rendered.ends_with('\n') {
        config.rendered.push('\n');
    }
    config.restore_original = false;
    config.doc = parse_doc(&config.path, &config.rendered)?;
    Ok(())
}

impl CodexConfig {
    fn append_addition(&mut self, addition: &str) -> Result<(), ProviderError> {
        append_owned(self, addition)
    }
}

fn explicit_table(parent: &str, child: &str, fields: &[(&str, Item)]) -> String {
    let mut doc = DocumentMut::new();
    let mut parent_table = Table::new();
    parent_table.set_implicit(true);
    let mut child_table = Table::new();
    child_table.set_implicit(false);
    for (key, value) in fields {
        child_table.insert(key, value.clone());
    }
    parent_table.insert(child, Item::Table(child_table));
    doc.insert(parent, Item::Table(parent_table));
    doc.to_string()
}

fn string_array(args: &[String]) -> Item {
    let mut array = toml_edit::Array::new();
    for arg in args {
        array.push(arg.as_str());
    }
    Item::Value(Value::Array(array))
}

fn child_table<'a>(doc: &'a DocumentMut, parent: &str, child: &str) -> Option<&'a Table> {
    doc.get(parent)
        .and_then(Item::as_table)
        .and_then(|table| table.get(child))
        .and_then(Item::as_table)
}

fn mcp_exists(doc: &DocumentMut, name: &str) -> bool {
    child_table(doc, "mcp_servers", name).is_some()
}

fn mcp_matches(doc: &DocumentMut, name: &str, command: &str, args: &[String]) -> bool {
    let Some(table) = child_table(doc, "mcp_servers", name) else {
        return false;
    };
    let Some(found) = table.get("command").and_then(Item::as_str) else {
        return false;
    };
    if found != command {
        return false;
    }
    match table.get("args").and_then(Item::as_array) {
        None => args.is_empty(),
        Some(array) => array_eq(array, args),
    }
}

fn project_exists(doc: &DocumentMut, project: &str) -> bool {
    child_table(doc, "projects", project).is_some()
}

fn project_matches(doc: &DocumentMut, project: &str, level: &str) -> bool {
    let Some(table) = child_table(doc, "projects", project) else {
        return false;
    };
    table.get("trust_level").and_then(Item::as_str) == Some(level)
}

fn array_eq(array: &toml_edit::Array, args: &[String]) -> bool {
    let mut found = Vec::new();
    for value in array {
        let Some(text) = value.as_str() else {
            return false;
        };
        found.push(text.to_owned());
    }
    found == args
}
