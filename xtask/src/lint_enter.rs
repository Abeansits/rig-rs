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
    if code.contains('\r') {
        hits.push(format!("{}:1: Enter", path.display()));
    }
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
    const BARE_CR: &[&str] = &[
        "\"\\r\"",
        "\"\\x0d\"",
        "\"\\u{d}\"",
        "\"\\u{000d}\"",
        "'\\r'",
        "'\\x0d'",
        "'\\u{d}'",
        "'\\u{000d}'",
    ];
    const NEWLINES: &[&str] = &["\"\\n\""];
    if NEEDLES.iter().any(|needle| compact.contains(needle))
        || compact.contains("\\r")
        || cr_constructor(compact)
        || decimal_as_char(compact)
        || hex_cr(compact)
    {
        return true;
    }
    if BARE_CR.iter().any(|needle| compact.contains(needle)) {
        return true;
    }
    // A const or a split newline still contains the escape. Flag it when the
    // file also builds a key from that value. A bare `"\r"` is already a hit.
    let has_newline = NEWLINES.iter().any(|escape| compact.contains(escape));
    has_newline && (compact.contains("Key::Literal") || compact.contains("send_keys"))
}

fn cr_constructor(compact: &str) -> bool {
    for head in ["char::from(", "char::from_u32("] {
        let mut rest = compact;
        while let Some(pos) = rest.find(head) {
            let after = rest.get(pos.saturating_add(head.len())..).unwrap_or("");
            if cr_arg(after) {
                return true;
            }
            rest = after;
        }
    }
    false
}

fn cr_arg(after: &str) -> bool {
    after.starts_with("13)")
        || after.starts_with("13u8)")
        || after.starts_with("0x0d)")
        || after.starts_with("0x0D)")
        || after.starts_with("0xd)")
        || after.starts_with("0xD)")
}

fn decimal_as_char(compact: &str) -> bool {
    let mut rest = compact;
    while let Some(pos) = rest.find("13aschar") {
        let before = rest.get(..pos).unwrap_or("");
        let prev_is_digit = before
            .chars()
            .next_back()
            .is_some_and(|ch| ch.is_ascii_digit());
        if !prev_is_digit {
            return true;
        }
        rest = rest.get(pos.saturating_add(1)..).unwrap_or("");
    }
    false
}

fn hex_cr(compact: &str) -> bool {
    for needle in ["0x0d", "0x0D", "0xd", "0xD"] {
        let mut rest = compact;
        while let Some(pos) = rest.find(needle) {
            let after = rest.get(pos.saturating_add(needle.len())..).unwrap_or("");
            if after.starts_with("aschar")
                || !after
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_ascii_hexdigit())
            {
                return true;
            }
            rest = after;
        }
    }
    false
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
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut index = 0usize;
    while index < chars.len() {
        if let Some(end) = raw_string_end(&chars, index) {
            push_range(&chars, index, end, &mut out);
            index = end;
            continue;
        }
        if chars.get(index).copied() == Some('"') {
            let end = quoted_end(&chars, index);
            push_range(&chars, index, end, &mut out);
            index = end;
            continue;
        }
        if chars.get(index).copied() == Some('/')
            && chars.get(index.saturating_add(1)).copied() == Some('/')
        {
            index = index.saturating_add(2);
            while let Some(ch) = chars.get(index).copied() {
                index = index.saturating_add(1);
                if ch == '\n' {
                    out.push('\n');
                    break;
                }
            }
            continue;
        }
        if chars.get(index).copied() == Some('/')
            && chars.get(index.saturating_add(1)).copied() == Some('*')
        {
            index = index.saturating_add(2);
            while index < chars.len() {
                if chars.get(index).copied() == Some('*')
                    && chars.get(index.saturating_add(1)).copied() == Some('/')
                {
                    index = index.saturating_add(2);
                    break;
                }
                index = index.saturating_add(1);
            }
            continue;
        }
        if let Some(ch) = chars.get(index).copied() {
            out.push(ch);
        }
        index = index.saturating_add(1);
    }
    out
}

fn push_range(chars: &[char], start: usize, end: usize, out: &mut String) {
    let mut index = start;
    while index < end {
        if let Some(ch) = chars.get(index).copied() {
            out.push(ch);
        }
        index = index.saturating_add(1);
    }
}

fn raw_string_end(chars: &[char], index: usize) -> Option<usize> {
    let hashes = raw_hashes(chars, index)?;
    let mut cursor = index.saturating_add(1);
    while chars.get(cursor).copied() == Some('#') {
        cursor = cursor.saturating_add(1);
    }
    if chars.get(cursor).copied() != Some('"') {
        return None;
    }
    cursor = cursor.saturating_add(1);
    loop {
        if chars.get(cursor).is_none() {
            return Some(chars.len());
        }
        if chars.get(cursor).copied() == Some('"')
            && hash_run(chars, cursor.saturating_add(1), hashes)
        {
            return Some(cursor.saturating_add(1).saturating_add(hashes));
        }
        cursor = cursor.saturating_add(1);
    }
}

fn raw_hashes(chars: &[char], index: usize) -> Option<usize> {
    if chars.get(index).copied() != Some('r') {
        return None;
    }
    if let Some(prev) = index.checked_sub(1).and_then(|at| chars.get(at).copied()) {
        if prev == 'b' || prev == 'c' {
            if let Some(before) = index.checked_sub(2).and_then(|at| chars.get(at).copied())
                && (before.is_ascii_alphanumeric() || before == '_')
            {
                return None;
            }
        } else if prev.is_ascii_alphanumeric() || prev == '_' {
            return None;
        }
    }
    let mut cursor = index.saturating_add(1);
    let mut hashes = 0usize;
    while chars.get(cursor).copied() == Some('#') {
        hashes = hashes.saturating_add(1);
        cursor = cursor.saturating_add(1);
    }
    (chars.get(cursor).copied() == Some('"')).then_some(hashes)
}

fn hash_run(chars: &[char], start: usize, hashes: usize) -> bool {
    let mut seen = 0usize;
    while seen < hashes {
        if chars.get(start.saturating_add(seen)).copied() != Some('#') {
            return false;
        }
        seen = seen.saturating_add(1);
    }
    true
}

fn quoted_end(chars: &[char], index: usize) -> usize {
    let mut cursor = index.saturating_add(1);
    while let Some(ch) = chars.get(cursor).copied() {
        cursor = cursor.saturating_add(1);
        if ch == '\\' {
            if chars.get(cursor).is_some() {
                cursor = cursor.saturating_add(1);
            }
            continue;
        }
        if ch == '"' {
            break;
        }
    }
    cursor
}

fn relative(path: &Path, root: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
}
