use std::collections::HashMap;
use std::path::{Path, PathBuf};

use regex::Regex;

/// Workspace root next to this crate.
#[must_use]
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Fail when sources build a banned attachment format, set `window-size` with
/// a short flag containing `g`, or spawn a shell from `crewmux-mux`.
pub fn lint_mux_at(root: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    // Crate trees only. A tmux source build (`tmux-3.3a/tmux.1.mdoc`) must not
    // be scanned; that man page names a format this lint rejects.
    for name in ["crates", "xtask"] {
        walk(&root.join(name), &mut files)?;
    }
    let needle = attached_needle();
    let command = command_re()?;
    let binding = binding_re()?;
    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        check_file(&path, &text, &needle, &command, &binding)?;
    }
    Ok(())
}

fn attached_needle() -> String {
    String::from_utf8(vec![
        115, 101, 115, 115, 105, 111, 110, 95, 97, 116, 116, 97, 99, 104, 101, 100,
    ])
    .unwrap_or_default()
}

fn check_file(
    path: &Path,
    text: &str,
    needle: &str,
    command: &Regex,
    binding: &Regex,
) -> Result<(), String> {
    let scanned = strip_clients_docs(path, text);
    if scanned.contains(needle) || joins_contain(&scanned, needle) {
        return Err(format!(
            "{} contains the banned attachment format",
            path.display()
        ));
    }
    if window_size_violation(path, &scanned) {
        return Err(format!("{} sets window size globally", path.display()));
    }
    if path_is_mux(path) {
        check_shells(path, &scanned, command, binding)?;
    }
    Ok(())
}

fn strip_clients_docs(path: &Path, text: &str) -> String {
    if !path.ends_with("crates/crewmux-mux/src/clients.rs") {
        return text.to_owned();
    }
    text.lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("//!") && !trimmed.starts_with("///")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn joins_contain(text: &str, needle: &str) -> bool {
    concat_joins(text)
        .into_iter()
        .chain(adjacent_runs(text))
        .any(|joined| joined.contains(needle))
}

fn window_size_violation(path: &Path, text: &str) -> bool {
    if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
        return path_is_mux(path)
            && concat_joins(text)
                .into_iter()
                .chain(adjacent_runs(text))
                .chain(array_groups(text))
                .any(|joined| global_window_size(&joined));
    }
    text.lines().any(global_window_size)
}

fn global_window_size(text: &str) -> bool {
    text.contains("window-size") && text.split_whitespace().any(short_flag_has_g)
}

fn short_flag_has_g(token: &str) -> bool {
    let Some(flags) = token.strip_prefix('-') else {
        return false;
    };
    !flags.is_empty() && flags.chars().all(|ch| ch.is_ascii_alphabetic()) && flags.contains('g')
}

fn array_groups(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut index = 0usize;
    let mut levels: Vec<Vec<String>> = Vec::new();
    while index < bytes.len() {
        if bytes.get(index) == Some(&b'/') && bytes.get(index.saturating_add(1)) == Some(&b'/') {
            index = skip_line(bytes, index);
            continue;
        }
        if bytes.get(index) == Some(&b'/') && bytes.get(index.saturating_add(1)) == Some(&b'*') {
            index = skip_block(bytes, index);
            continue;
        }
        if let Some(next) = skip_char_literal(bytes, index) {
            index = next;
            continue;
        }
        if bytes.get(index) == Some(&b'"')
            && let Some((value, next)) = parse_string(bytes, index)
        {
            if let Some(level) = levels.last_mut() {
                level.push(value);
            }
            index = next;
            continue;
        }
        if bytes.get(index) == Some(&b'[') {
            levels.push(Vec::new());
            index = index.saturating_add(1);
            continue;
        }
        if bytes.get(index) == Some(&b']')
            && let Some(literals) = levels.pop()
            && !literals.is_empty()
        {
            out.push(literals.join(" "));
            index = index.saturating_add(1);
            continue;
        }
        index = index.saturating_add(1);
    }
    out
}

fn skip_line(bytes: &[u8], index: usize) -> usize {
    let rest = bytes.get(index.saturating_add(2)..).unwrap_or_default();
    let at = rest
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(rest.len());
    index.saturating_add(3).saturating_add(at)
}

fn skip_block(bytes: &[u8], index: usize) -> usize {
    let mut index = index.saturating_add(2);
    while index < bytes.len()
        && (bytes.get(index) != Some(&b'*') || bytes.get(index.saturating_add(1)) != Some(&b'/'))
    {
        index = index.saturating_add(1);
    }
    index.saturating_add(2)
}

fn skip_char_literal(bytes: &[u8], index: usize) -> Option<usize> {
    if bytes.get(index) != Some(&b'\'') {
        return None;
    }
    let escaped = bytes.get(index.saturating_add(1)) == Some(&b'\\');
    let end = index.saturating_add(if escaped { 3 } else { 2 });
    (bytes.get(end) == Some(&b'\'')).then_some(end.saturating_add(1))
}

fn path_is_mux(path: &Path) -> bool {
    path.to_string_lossy().contains("crates/crewmux-mux/")
}

fn concat_joins(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = text.get(from..).and_then(|rest| rest.find("concat!")) {
        let after = from.saturating_add(rel).saturating_add("concat!".len());
        let Some(tail) = text.get(after..) else {
            break;
        };
        let Some(open_rel) = tail.find('(') else {
            break;
        };
        if !tail
            .get(..open_rel)
            .unwrap_or("")
            .chars()
            .all(char::is_whitespace)
        {
            from = after.saturating_add(1);
            continue;
        }
        let open_at = after.saturating_add(open_rel);
        let Some(body) = paren_body(text, open_at) else {
            break;
        };
        let joined = literals_in(body);
        if !joined.is_empty() {
            out.push(joined);
        }
        from = open_at.saturating_add(body.len()).saturating_add(2);
    }
    out
}

fn adjacent_runs(text: &str) -> Vec<String> {
    let lits = locate_strings(text);
    let mut out = Vec::new();
    let mut index = 0usize;
    while index < lits.len() {
        let Some(mut end) = lits.get(index).map(|lit| lit.end) else {
            break;
        };
        let mut joined = lits
            .get(index)
            .map(|lit| lit.value.clone())
            .unwrap_or_default();
        index = index.saturating_add(1);
        while let Some(next) = lits.get(index) {
            let gap = text.get(end..next.start).unwrap_or("x");
            if !gap.chars().all(char::is_whitespace) {
                break;
            }
            joined.push_str(&next.value);
            end = next.end;
            index = index.saturating_add(1);
        }
        out.push(joined);
    }
    out
}

struct Lit {
    start: usize,
    end: usize,
    value: String,
}

fn literals_in(text: &str) -> String {
    locate_strings(text)
        .into_iter()
        .map(|lit| lit.value)
        .collect()
}

fn locate_strings(text: &str) -> Vec<Lit> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes.get(index) == Some(&b'"')
            && let Some((value, next)) = parse_string(bytes, index)
        {
            out.push(Lit {
                start: index,
                end: next,
                value,
            });
            index = next;
            continue;
        }
        index = index.saturating_add(1);
    }
    out
}

fn parse_string(bytes: &[u8], start: usize) -> Option<(String, usize)> {
    let mut index = start.saturating_add(1);
    let mut value = String::new();
    while let Some(byte) = bytes.get(index).copied() {
        if byte == b'\\' {
            let escaped = bytes.get(index.saturating_add(1)).copied()?;
            value.push(char::from(escaped));
            index = index.saturating_add(2);
            continue;
        }
        if byte == b'"' {
            return Some((value, index.saturating_add(1)));
        }
        value.push(char::from(byte));
        index = index.saturating_add(1);
    }
    None
}

fn paren_body(text: &str, open_at: usize) -> Option<&str> {
    let bytes = text.as_bytes();
    if bytes.get(open_at) != Some(&b'(') {
        return None;
    }
    let mut depth = 0i32;
    let mut index = open_at;
    let mut in_string = false;
    let mut escape = false;
    while let Some(byte) = bytes.get(index).copied() {
        if in_string {
            if escape {
                escape = false;
            } else if byte == b'\\' {
                escape = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
        } else if byte == b'(' {
            depth = depth.saturating_add(1);
        } else if byte == b')' {
            depth -= 1;
            if depth == 0 {
                return text.get(open_at.saturating_add(1)..index);
            }
        }
        index = index.saturating_add(1);
    }
    None
}

fn command_re() -> Result<Regex, String> {
    Regex::new(r#"Command\s*::\s*new\s*\(\s*("(?:\\.|[^"\\])*"|[A-Za-z_][A-Za-z0-9_]*)"#)
        .map_err(|err| err.to_string())
}

fn binding_re() -> Result<Regex, String> {
    Regex::new(
        r#"(?m)(?:let\s+(?:mut\s+)?|const\s+)([A-Za-z_][A-Za-z0-9_]*)\s*(?::[^=\n]+)?=\s*("(?:\\.|[^"\\])*")"#,
    )
    .map_err(|err| err.to_string())
}

fn check_shells(path: &Path, text: &str, command: &Regex, binding: &Regex) -> Result<(), String> {
    let mut bound = HashMap::<String, String>::new();
    for cap in binding.captures_iter(text) {
        let Some(name) = cap.get(1) else { continue };
        let Some(raw) = cap.get(2) else { continue };
        bound.insert(name.as_str().to_owned(), raw.as_str().to_owned());
    }
    for cap in command.captures_iter(text) {
        let Some(arg) = cap.get(1) else { continue };
        let raw = arg.as_str();
        let resolved = if raw.starts_with('"') {
            raw.to_owned()
        } else {
            bound.get(raw).cloned().unwrap_or_default()
        };
        if is_shell_token(&resolved) {
            return Err(format!(
                "{} invokes a shell via Command::new ({raw})",
                path.display()
            ));
        }
    }
    Ok(())
}

fn is_shell_token(raw: &str) -> bool {
    let mut decoded = String::new();
    let mut chars = raw.trim().trim_matches('"').chars();
    while let Some(ch) = chars.next() {
        decoded.push(if ch == '\\' {
            chars.next().unwrap_or('\\')
        } else {
            ch
        });
    }
    let base = decoded
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(decoded.as_str());
    matches!(base, "sh" | "bash" | "dash" | "zsh" | "ksh")
}

fn walk(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    if !dir.is_dir() {
        return Ok(());
    }
    let entries = std::fs::read_dir(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| err.to_string())?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "target" || name == ".git" {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            walk(&path, files)?;
        } else {
            files.push(path);
        }
    }
    Ok(())
}
