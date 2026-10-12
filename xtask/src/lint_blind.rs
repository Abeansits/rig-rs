//! Reject blind Enter, quit keys, and digits in adapter `send_keys` calls.
//!
//! Enter is allowed only in `DialogDriver` (after the cursor is verified) and
//! in Codex submit (after the composer is ready). `q`, Ctrl-C, Escape, digit
//! literals, and raw tmux `C-m` are not sent into a pane. The scan sees
//! multi-line paths and `Key` aliases, not only a single source line.

use std::fs;
use std::path::Path;

use crate::error::{Error, io_err};

const ENTER_ALLOWED: &[&str] = &[
    "crates/crewmux-adapters/src/dialog/driver.rs",
    "crates/crewmux-adapters/src/codex/submit.rs",
];

const KEY_KINDS: &[&str] = &["Enter", "CtrlC", "Escape", "Literal"];

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
    let blank = blank_comments(text);
    let names = aliases(&blank);
    for hit in key_hits(&blank, &names) {
        report_key(&display, &blank, enter_ok, &hit, hits);
    }
    for at in raw_cm(&blank) {
        let line = line_no(&blank, at);
        hits.push(format!("{display}:{line}: raw C-m is a blind Enter"));
    }
}

struct Hit {
    at: usize,
    kind: String,
}

fn report_key(display: &str, text: &str, enter_ok: bool, hit: &Hit, hits: &mut Vec<String>) {
    let line = line_no(text, hit.at);
    match hit.kind.as_str() {
        "Literal" if carriage_return_in(literal_window(text, hit.at)) => {
            hits.push(format!(
                "{display}:{line}: carriage return literal is a blind Enter"
            ));
        }
        "Enter" if !enter_ok => {
            hits.push(format!(
                "{display}:{line}: Enter is only sent from DialogDriver or Codex submit"
            ));
        }
        "CtrlC" | "Escape" | "Literal" => {
            hits.push(format!(
                "{display}:{line}: forbidden key outside a ready composer"
            ));
        }
        _ => {}
    }
}

fn aliases(text: &str) -> Vec<String> {
    let mut names = vec!["Key".to_owned()];
    push_renames(text, &mut names);
    push_type_aliases(text, &mut names);
    names
}

fn push_renames(text: &str, names: &mut Vec<String>) {
    let mut search = 0;
    while let Some(at) = find_word_from(text, search, "Key") {
        let after = slice_from(text, at.saturating_add(3)).trim_start();
        if let Some(name) = rename_target(after)
            && !names.iter().any(|have| have == name)
        {
            names.push(name.to_owned());
        }
        search = at.saturating_add(3);
    }
}

fn rename_target(after: &str) -> Option<&str> {
    let rest = after.strip_prefix("as")?;
    let next = rest.chars().next()?;
    if !next.is_whitespace() {
        return None;
    }
    let name = leading_ident(rest.trim_start());
    if name.is_empty() { None } else { Some(name) }
}

fn push_type_aliases(text: &str, names: &mut Vec<String>) {
    let mut search = 0;
    while let Some(at) = find_word_from(text, search, "type") {
        let after = slice_from(text, at.saturating_add(4)).trim_start();
        let name = leading_ident(after);
        if !name.is_empty() {
            let after_name = slice_from(after, name.len()).trim_start();
            if let Some(rhs) = after_name.strip_prefix('=')
                && rhs_is_key(rhs)
                && !names.iter().any(|have| have == name)
            {
                names.push(name.to_owned());
            }
        }
        search = at.saturating_add(4);
    }
}

fn rhs_is_key(rhs: &str) -> bool {
    let end = rhs.find(';').unwrap_or(rhs.len());
    let Some(rhs) = rhs.get(..end) else {
        return false;
    };
    let Some(last) = rhs.split_whitespace().last() else {
        return false;
    };
    last.rsplit("::").next() == Some("Key")
}

fn key_hits(text: &str, names: &[String]) -> Vec<Hit> {
    let mut hits = Vec::new();
    let mut search = 0;
    while let Some(rel) = slice_from(text, search).find("::") {
        let at = search.saturating_add(rel);
        let name = trailing_ident(text.get(..at).unwrap_or(""));
        let after = slice_from(text, at.saturating_add(2));
        let trimmed = after.trim_start();
        let kind_at = at
            .saturating_add(2)
            .saturating_add(after.len().saturating_sub(trimmed.len()));
        let kind = leading_ident(trimmed);
        if names.iter().any(|candidate| candidate == name) && KEY_KINDS.contains(&kind) {
            hits.push(Hit {
                at: kind_at,
                kind: kind.to_owned(),
            });
        }
        search = at.saturating_add(2);
    }
    hits
}

fn raw_cm(text: &str) -> Vec<usize> {
    let mut hits = Vec::new();
    let mut search = 0;
    while let Some(rel) = slice_from(text, search).find("C-m") {
        let at = search.saturating_add(rel);
        if bounded(text, at, 3) {
            hits.push(at);
        }
        search = at.saturating_add(3);
    }
    hits
}

fn bounded(text: &str, at: usize, len: usize) -> bool {
    let before = at == 0 || text.as_bytes().get(at - 1).is_some_and(|b| !is_ident(*b));
    let after = text
        .as_bytes()
        .get(at.saturating_add(len))
        .is_none_or(|b| !is_ident(*b));
    before && after
}

fn literal_window(text: &str, at: usize) -> &str {
    let rest = slice_from(text, at);
    let cap = rest.len().min(240);
    let end = rest.find(';').map_or(cap, |index| index.min(cap));
    rest.get(..end).unwrap_or("")
}

fn carriage_return_in(window: &str) -> bool {
    if window.chars().any(|ch| ch == '\r') {
        return true;
    }
    let compact: String = window.chars().filter(|ch| !ch.is_whitespace()).collect();
    let lower = compact.to_ascii_lowercase();
    lower.contains("\\r") || lower.contains("\\x0d") || unicode_cr(&lower)
}

fn unicode_cr(compact: &str) -> bool {
    let mut rest = compact;
    while let Some(index) = rest.find("\\u{") {
        let after = slice_from(rest, index.saturating_add(3));
        let Some(end) = after.find('}') else {
            return false;
        };
        let Some(hex) = after.get(..end) else {
            return false;
        };
        if hex.chars().all(|ch| ch.is_ascii_hexdigit()) && u32::from_str_radix(hex, 16) == Ok(0x0d)
        {
            return true;
        }
        rest = slice_from(after, end.saturating_add(1));
    }
    false
}

fn blank_comments(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if starts_with(bytes, index, b"//") {
            while index < bytes.len() && bytes.get(index) != Some(&b'\n') {
                out.push(b' ');
                index = index.saturating_add(1);
            }
            continue;
        }
        if starts_with(bytes, index, b"/*") {
            blank_block(bytes, &mut out, &mut index);
            continue;
        }
        let Some(byte) = bytes.get(index).copied() else {
            break;
        };
        out.push(byte);
        index = index.saturating_add(1);
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

fn blank_block(bytes: &[u8], out: &mut Vec<u8>, index: &mut usize) {
    out.push(b' ');
    out.push(b' ');
    *index = index.saturating_add(2);
    while *index < bytes.len() && !starts_with(bytes, *index, b"*/") {
        let byte = bytes.get(*index).copied().unwrap_or(b' ');
        out.push(if byte == b'\n' { b'\n' } else { b' ' });
        *index = index.saturating_add(1);
    }
    if *index < bytes.len() {
        out.push(b' ');
        *index = index.saturating_add(1);
    }
    if *index < bytes.len() {
        out.push(b' ');
        *index = index.saturating_add(1);
    }
}

fn starts_with(bytes: &[u8], index: usize, prefix: &[u8]) -> bool {
    bytes.get(index..index.saturating_add(prefix.len())) == Some(prefix)
}

fn find_word_from(text: &str, start: usize, word: &str) -> Option<usize> {
    let rest = text.get(start..)?;
    let rel = find_word(rest, word)?;
    Some(start.saturating_add(rel))
}

fn find_word(text: &str, word: &str) -> Option<usize> {
    let mut search = 0;
    while let Some(rel) = slice_from(text, search).find(word) {
        let at = search.saturating_add(rel);
        if bounded(text, at, word.len()) {
            return Some(at);
        }
        search = at.saturating_add(word.len());
    }
    None
}

fn trailing_ident(text: &str) -> &str {
    let trimmed = text.trim_end();
    let bytes = trimmed.as_bytes();
    let mut start = bytes.len();
    while start > 0 && bytes.get(start - 1).is_some_and(|byte| is_ident(*byte)) {
        start = start.saturating_sub(1);
    }
    let ident = trimmed.get(start..).unwrap_or("");
    if leading_ident(ident) == ident {
        ident
    } else {
        ""
    }
}

fn leading_ident(text: &str) -> &str {
    let bytes = text.as_bytes();
    let Some(first) = bytes.first().copied() else {
        return "";
    };
    if !first.is_ascii_alphabetic() && first != b'_' {
        return "";
    }
    let mut end = 1;
    while bytes.get(end).is_some_and(|byte| is_ident(*byte)) {
        end = end.saturating_add(1);
    }
    text.get(..end).unwrap_or("")
}

fn slice_from(text: &str, start: usize) -> &str {
    text.get(start..).unwrap_or("")
}

fn line_no(text: &str, offset: usize) -> usize {
    text.get(..offset)
        .unwrap_or(text)
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        .saturating_add(1)
}

fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}
