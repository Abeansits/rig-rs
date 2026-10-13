//! Legacy MCP names stay in `names.rs`. The MCP crate does not own state.

use std::fs;
use std::path::Path;

use crate::error::{Error, io_err};

const NAMES_REL: &str = "crates/crewmux-mcp/src/names.rs";
const MCP_CARGO: &str = "crates/crewmux-mcp/Cargo.toml";

/// The 18 legacy tool names. Kept here so the lint does not depend on the MCP crate.
const LEGACY_NAMES: [&str; 18] = [
    "rig_add",
    "rig_agent_validate",
    "rig_bind",
    "rig_bundle_inspect",
    "rig_capture",
    "rig_chatroom_send",
    "rig_chatroom_watch",
    "rig_discover",
    "rig_down",
    "rig_ps",
    "rig_restore",
    "rig_rig_nodes",
    "rig_rig_validate",
    "rig_send",
    "rig_snapshot_create",
    "rig_snapshot_list",
    "rig_status",
    "rig_up",
];

const STATE_CRATES: [&str; 4] = [
    "crewmux-store",
    "crewmux-mux",
    "crewmux-adapters",
    "crewmux-durability",
];

/// Fail when a legacy tool name appears outside the allow-list.
///
/// Scans `docs/`, `assets/skills/`, and `crates/**/src`. `names.rs` is the only
/// allow-listed source file.
///
/// # Errors
///
/// Returns [`Error`] when a name is found, or a directory cannot be read.
pub fn lint_mcp_legacy_names(root: &Path) -> Result<(), Error> {
    let mut hits = Vec::new();
    for rel in ["docs", "assets/skills", "crates"] {
        let dir = root.join(rel);
        if dir.is_dir() {
            walk(&dir, root, &mut hits)?;
        }
    }
    if hits.is_empty() {
        Ok(())
    } else {
        Err(Error::Message(format!(
            "lint-mcp-legacy-names: legacy tool names belong in {NAMES_REL}:\n{}",
            hits.join("\n")
        )))
    }
}

/// Fail when `crewmux-mcp` depends on a state crate.
///
/// Only the `[dependencies]` table counts. Dev-dependencies are tests.
///
/// # Errors
///
/// Returns [`Error`] when a forbidden dependency is present, or the manifest
/// cannot be read.
pub fn lint_mcp_state_path(root: &Path) -> Result<(), Error> {
    let path = root.join(MCP_CARGO);
    if !path.is_file() {
        return Ok(());
    }
    let text = fs::read_to_string(&path)
        .map_err(|source| io_err(format!("read {}", path.display()), source))?;
    let deps = dependencies_block(&text);
    let mut hits = Vec::new();
    for name in STATE_CRATES {
        if dep_key_present(&deps, name) {
            hits.push(name);
        }
    }
    if hits.is_empty() {
        Ok(())
    } else {
        Err(Error::Message(format!(
            "lint-mcp-state-path: crewmux-mcp must not depend on {} (found in {MCP_CARGO})",
            hits.join(", ")
        )))
    }
}

fn walk(dir: &Path, root: &Path, hits: &mut Vec<String>) -> Result<(), Error> {
    let entries =
        fs::read_dir(dir).map_err(|source| io_err(format!("read {}", dir.display()), source))?;
    for entry in entries {
        let entry = entry.map_err(|source| io_err("read directory entry", source))?;
        let path = entry.path();
        if path.is_dir() {
            if skip_dir(&path) {
                continue;
            }
            walk(&path, root, hits)?;
            continue;
        }
        consider(&path, root, hits)?;
    }
    Ok(())
}

fn skip_dir(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("target" | "tests" | ".git")
    )
}

fn consider(path: &Path, root: &Path, hits: &mut Vec<String>) -> Result<(), Error> {
    let rel = relative(path, root);
    if !in_scan(&rel) || rel == NAMES_REL {
        return Ok(());
    }
    let bytes =
        fs::read(path).map_err(|source| io_err(format!("read {}", path.display()), source))?;
    if bytes.contains(&0) {
        return Ok(());
    }
    let Ok(text) = String::from_utf8(bytes) else {
        return Ok(());
    };
    for name in LEGACY_NAMES {
        if bounded(&text, name) {
            hits.push(format!("{rel}: {name}"));
        }
    }
    Ok(())
}

fn in_scan(rel: &str) -> bool {
    rel.starts_with("docs/")
        || rel.starts_with("assets/skills/")
        || (rel.starts_with("crates/") && rel.contains("/src/"))
}

fn relative(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn bounded(text: &str, name: &str) -> bool {
    let mut rest = text;
    while let Some(pos) = rest.find(name) {
        let end = pos.saturating_add(name.len());
        let Some(prefix) = rest.get(..pos) else {
            return false;
        };
        let Some(suffix) = rest.get(end..) else {
            return false;
        };
        let before = prefix.chars().next_back();
        let after = suffix.chars().next();
        if before.is_none_or(|ch| !is_ident(ch)) && after.is_none_or(|ch| !is_ident(ch)) {
            return true;
        }
        rest = suffix;
    }
    false
}

fn is_ident(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn dependencies_block(text: &str) -> String {
    let mut in_deps = false;
    let mut out = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let header = trimmed.trim_matches(['[', ']']).trim();
            in_deps = header == "dependencies" || header.starts_with("dependencies.");
            if let Some(name) = header.strip_prefix("dependencies.") {
                out.push_str(name);
                out.push_str(" =\n");
            }
            continue;
        }
        if in_deps {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn dep_key_present(block: &str, name: &str) -> bool {
    block.lines().any(|line| {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix(name) else {
            return false;
        };
        let rest = rest.trim_start();
        rest.starts_with('=') || rest.starts_with('.')
    })
}
