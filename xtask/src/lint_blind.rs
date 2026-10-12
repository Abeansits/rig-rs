//! Reject blind Enter, quit keys, and digits in adapter `send_keys` calls.
//!
//! Enter is allowed only in `DialogDriver` (after the cursor is verified) and
//! in Codex submit (after the composer is ready). `q`, Ctrl-C, Escape, and
//! digit literals are not sent into a pane.

use std::fs;
use std::path::Path;

use crate::error::{Error, io_err};

const ENTER_ALLOWED: &[&str] = &[
    "crates/crewmux-adapters/src/dialog/driver.rs",
    "crates/crewmux-adapters/src/codex/submit.rs",
];

/// Fail when an adapter source sends a forbidden key.
///
/// # Errors
///
/// Returns [`Error`] when a scanned file contains a forbidden key, or a
/// directory cannot be read.
pub fn lint_blind_enter(root: &Path) -> Result<(), Error> {
    let mut hits = Vec::new();
    let dir = root.join("crates/crewmux-adapters/src");
    scan(&dir, root, &mut hits)?;
    if hits.is_empty() {
        Ok(())
    } else {
        Err(Error::Message(format!(
            "lint-blind-enter:\n{}",
            hits.join("\n")
        )))
    }
}

fn scan(dir: &Path, workspace: &Path, hits: &mut Vec<String>) -> Result<(), Error> {
    let entries =
        fs::read_dir(dir).map_err(|source| io_err(format!("read {}", dir.display()), source))?;
    for entry in entries {
        let entry = entry.map_err(|source| io_err(format!("read {}", dir.display()), source))?;
        let path = entry.path();
        if path.is_dir() {
            scan(&path, workspace, hits)?;
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let text = fs::read_to_string(&path)
            .map_err(|source| io_err(format!("read {}", path.display()), source))?;
        let relative = path.strip_prefix(workspace).unwrap_or(&path);
        check_file(relative, &text, hits);
    }
    Ok(())
}

fn check_file(path: &Path, text: &str, hits: &mut Vec<String>) {
    let display = path.display().to_string();
    let enter_ok = ENTER_ALLOWED.contains(&display.as_str());
    for (index, line) in text.lines().enumerate() {
        if is_comment(line) {
            continue;
        }
        let line_no = index.saturating_add(1);
        if forbidden_key(line) {
            hits.push(format!(
                "{display}:{line_no}: forbidden key outside a ready composer"
            ));
        }
        if line.contains("Key::Enter") && !enter_ok {
            hits.push(format!(
                "{display}:{line_no}: Enter is only sent from DialogDriver or Codex submit"
            ));
        }
    }
}

fn is_comment(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("//")
}

fn forbidden_key(line: &str) -> bool {
    line.contains("Key::CtrlC") || line.contains("Key::Escape") || line.contains("Key::Literal")
}
