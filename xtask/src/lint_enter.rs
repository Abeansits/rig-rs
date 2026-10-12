//! Adapters may send Enter only from submit and the dialog driver.

use std::fs;
use std::path::Path;

use crate::error::{Error, io_err};

const ALLOW_ENTER: &[&str] = &[
    "crates/crewmux-adapters/src/dialog/driver.rs",
    "crates/crewmux-adapters/src/claude/submit.rs",
    "crates/crewmux-adapters/src/codex/submit.rs",
];

/// Fail when adapter source sends Enter, quit, or a menu digit outside the gates.
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
    let code = strip_comments(&text);
    let compact: String = code.chars().filter(|ch| !ch.is_whitespace()).collect();
    if !allow_enter && (compact.contains("Key::Enter") || compact.contains("Key::*")) {
        hits.push(format!("{}:1: Enter", path.display()));
    }
    if !allow_enter && (compact.contains("\"C-m\"") || compact.contains("\"Enter\"")) {
        hits.push(format!("{}:1: Enter", path.display()));
    }
    if carriage_return_literal(&compact) {
        hits.push(format!("{}:1: Enter", path.display()));
    }
    if compact.contains("Key::CtrlC") || compact.contains("Key::Escape") {
        hits.push(format!("{}:1: quit key", path.display()));
    }
    if forbidden_literal(&compact) {
        hits.push(format!("{}:1: menu literal", path.display()));
    }
    Ok(())
}

fn carriage_return_literal(compact: &str) -> bool {
    const NEEDLES: &[&str] = &[
        "Key::Literal(\"\\r\")",
        "Key::Literal('\\r')",
        "Key::Literal(\"\r\")",
        "Key::Literal('\r')",
        "Key::Literal(\"\\n\")",
        "Key::Literal(\"\\x0d\")",
        "Key::Literal(\"\\u{d}\")",
        "Key::Literal(\"\\u{000d}\")",
        "\"\\r\".into()",
        "\"\\n\".into()",
        "\"\\x0d\".into()",
        "\"\\u{d}\".into()",
        "concat!(\"\\r\"",
        "concat!(\"\\n\"",
        "concat!(\"\\x0d\"",
        "concat!(\"\\u{d}\"",
    ];
    const ESCAPES: &[&str] = &[
        "\"\\r\"",
        "\"\\n\"",
        "\"\\x0d\"",
        "\"\\u{d}\"",
        "\"\\u{000d}\"",
    ];
    if NEEDLES.iter().any(|needle| compact.contains(needle)) {
        return true;
    }
    // A const or a split literal still contains the escape. Flag it when the
    // file also builds a key from that value.
    let has_escape = ESCAPES.iter().any(|escape| compact.contains(escape));
    has_escape && (compact.contains("Key::Literal") || compact.contains("send_keys"))
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

fn strip_comments(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '/' && chars.peek() == Some(&'/') {
            chars.next();
            for next in chars.by_ref() {
                if next == '\n' {
                    out.push('\n');
                    break;
                }
            }
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(next) = chars.next() {
                if next == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

fn relative(path: &Path, root: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
}
