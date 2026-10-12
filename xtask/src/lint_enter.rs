//! Adapters may send Enter only from submit and the dialog driver.

use std::fs;
use std::path::Path;

use crate::error::{Error, io_err};

const ALLOW_ENTER: &[&str] = &[
    "crates/crewmux-adapters/src/claude/submit.rs",
    "crates/crewmux-adapters/src/dialog/drive.rs",
];

/// Fail when adapter source sends Enter, quit, or a menu digit outside the two gates.
///
/// # Errors
///
/// Returns [`Error`] when a forbidden key is found, or a directory cannot be read.
pub fn lint_blind_enter(root: &Path) -> Result<(), Error> {
    let dir = root.join("crates/crewmux-adapters/src");
    let mut hits = Vec::new();
    if dir.is_dir() {
        walk(&dir, root, &mut hits)?;
    }
    if hits.is_empty() {
        Ok(())
    } else {
        Err(Error::Message(format!(
            "lint-blind-enter: Enter is only allowed in submit and the dialog driver; quit keys are never allowed:\n{}",
            hits.join("\n")
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
            walk(&path, root, hits)?;
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        scan(&path, root, hits)?;
    }
    Ok(())
}

fn scan(path: &Path, root: &Path, hits: &mut Vec<String>) -> Result<(), Error> {
    let text = fs::read_to_string(path)
        .map_err(|source| io_err(format!("read {}", path.display()), source))?;
    let allow_enter = relative(path, root).is_some_and(|rel| ALLOW_ENTER.contains(&rel.as_str()));
    for (index, line) in text.lines().enumerate() {
        let code = strip_comment(line);
        let compact: String = code.chars().filter(|ch| !ch.is_whitespace()).collect();
        let line_no = index.saturating_add(1);
        if !allow_enter && compact.contains("Key::Enter") {
            hits.push(format!("{}:{line_no}: Enter", path.display()));
        }
        if carriage_return_literal(&compact) {
            hits.push(format!("{}:{line_no}: Enter", path.display()));
        }
        if compact.contains("Key::CtrlC") || compact.contains("Key::Escape") {
            hits.push(format!("{}:{line_no}: quit key", path.display()));
        }
        if forbidden_literal(&compact) {
            hits.push(format!("{}:{line_no}: menu literal", path.display()));
        }
    }
    Ok(())
}

fn carriage_return_literal(compact: &str) -> bool {
    compact.contains("Key::Literal(\"\\r\")")
        || compact.contains("Key::Literal('\\r')")
        || compact.contains("Key::Literal(\"\r\")")
        || compact.contains("Key::Literal('\r')")
}

fn forbidden_literal(compact: &str) -> bool {
    for needle in ["Key::Literal(\"", "Key::Literal('"] {
        let mut rest = compact;
        while let Some(pos) = rest.find(needle) {
            let after = rest.get(pos.saturating_add(needle.len())..).unwrap_or("");
            if let Some(ch) = after.chars().next()
                && (ch == 'q' || ch == 'y' || ch == 'n' || ch.is_ascii_digit())
            {
                return true;
            }
            rest = after;
        }
    }
    false
}

fn strip_comment(line: &str) -> &str {
    line.split("//").next().unwrap_or(line)
}

fn relative(path: &Path, root: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
}
