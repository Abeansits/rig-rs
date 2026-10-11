//! Source lints for the Claude user-config guard and the tmux template.

use std::fs;
use std::path::Path;

use crate::error::{Error, io_err};

const TRUST_REL: &str = "crates/crewmux-provider/src/claude/trust.rs";

/// Fail when a non-test Rust file outside the trust module names the Claude user config.
///
/// # Errors
///
/// Returns [`Error`] when a forbidden file contains the basename, or a directory
/// cannot be read.
pub fn lint_claude_json(root: &Path) -> Result<(), Error> {
    let allowed = root.join(TRUST_REL);
    let mut hits = Vec::new();
    walk_rs(root, &allowed, &mut hits)?;
    if hits.is_empty() {
        Ok(())
    } else {
        Err(Error::Message(format!(
            "lint-claude-json: only {TRUST_REL} may mention the Claude user config:\n{}",
            hits.join("\n")
        )))
    }
}

/// Fail when a provider source line sets a global window size.
///
/// # Errors
///
/// Returns [`Error`] when a scanned line contains both tokens, or a directory
/// cannot be read.
pub fn lint_window_size(root: &Path) -> Result<(), Error> {
    let mut hits = Vec::new();
    scan_tree(&root.join("crates/crewmux-provider/src"), &mut hits)?;
    scan_tree(&root.join("assets"), &mut hits)?;
    if hits.is_empty() {
        Ok(())
    } else {
        Err(Error::Message(format!(
            "lint-window-size: global window size is forbidden:\n{}",
            hits.join("\n")
        )))
    }
}

fn walk_rs(root: &Path, allowed: &Path, hits: &mut Vec<String>) -> Result<(), Error> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if skip_dir(&dir) {
            continue;
        }
        let entries = fs::read_dir(&dir)
            .map_err(|source| io_err(format!("read {}", dir.display()), source))?;
        for entry in entries {
            let entry = entry.map_err(|source| io_err("read directory entry", source))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            consider_rs(&path, allowed, hits)?;
        }
    }
    Ok(())
}

fn consider_rs(path: &Path, allowed: &Path, hits: &mut Vec<String>) -> Result<(), Error> {
    if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
        return Ok(());
    }
    if skip_dir(path) || path == allowed || is_lint_source(path) {
        return Ok(());
    }
    let text = fs::read_to_string(path)
        .map_err(|source| io_err(format!("read {}", path.display()), source))?;
    if mentions_user_config(&text) {
        hits.push(path.display().to_string());
    }
    Ok(())
}

fn is_lint_source(path: &Path) -> bool {
    path.ends_with("xtask/src/lint.rs")
}

/// Contiguous basename, or `concat!` pieces that join into it.
fn mentions_user_config(text: &str) -> bool {
    let banned = user_config_name();
    if text.contains(&banned) {
        return true;
    }
    concat_invocations(text)
        .iter()
        .any(|joined| joined.contains(&banned))
}

fn user_config_name() -> String {
    let mut name = String::from(".claude");
    name.push('.');
    name.push_str("json");
    name
}

fn concat_invocations(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("concat!") {
        let after = rest
            .get(start.saturating_add("concat!".len())..)
            .unwrap_or("");
        let Some(open) = after.find('(') else {
            break;
        };
        let args = after.get(open.saturating_add(1)..).unwrap_or("");
        let Some(close) = args.find(')') else {
            break;
        };
        let inside = args.get(..close).unwrap_or("");
        found.push(join_string_literals(inside));
        rest = args.get(close.saturating_add(1)..).unwrap_or("");
    }
    found
}

fn join_string_literals(args: &str) -> String {
    let mut out = String::new();
    let bytes = args.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes.get(index) != Some(&b'"') {
            index = index.saturating_add(1);
            continue;
        }
        index = index.saturating_add(1);
        let start = index;
        while index < bytes.len() && bytes.get(index) != Some(&b'"') {
            if bytes.get(index) == Some(&b'\\') {
                index = index.saturating_add(1);
            }
            index = index.saturating_add(1);
        }
        if let Some(piece) = args.get(start..index) {
            out.push_str(piece);
        }
        index = index.saturating_add(1);
    }
    out
}

fn scan_tree(root: &Path, hits: &mut Vec<String>) -> Result<(), Error> {
    if !root.exists() {
        return Ok(());
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if !dir.is_dir() {
            scan_file(&dir, hits)?;
            continue;
        }
        if skip_dir(&dir) {
            continue;
        }
        let entries = fs::read_dir(&dir)
            .map_err(|source| io_err(format!("read {}", dir.display()), source))?;
        for entry in entries {
            let entry = entry.map_err(|source| io_err("read directory entry", source))?;
            stack.push(entry.path());
        }
    }
    Ok(())
}

fn scan_file(path: &Path, hits: &mut Vec<String>) -> Result<(), Error> {
    if skip_dir(path) {
        return Ok(());
    }
    let text = fs::read_to_string(path)
        .map_err(|source| io_err(format!("read {}", path.display()), source))?;
    for (index, line) in text.lines().enumerate() {
        if sets_global_window_size(line) {
            let line_no = index.saturating_add(1);
            hits.push(format!("{}:{line_no}", path.display()));
        }
    }
    Ok(())
}

fn sets_global_window_size(line: &str) -> bool {
    let size = line.contains("window-size");
    let global = line.contains("-g");
    size && global
}

fn skip_dir(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some("tests" | "target" | ".git")
        )
    })
}
