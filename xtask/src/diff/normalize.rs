//! Mask volatile tokens so two hermetic runs compare byte-for-byte.

use std::collections::BTreeMap;
use std::path::PathBuf;

use regex::{Captures, Regex};

use super::types::{NormRule, Observation};
use crate::error::Error;

const ULID_RE: &str = r"(?i)[0-9A-HJKMNP-TV-Z]{26}";
const TIMESTAMP_RE: &str =
    r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})";
const SOCKET_RE: &str = r#"[^\s"'<>]+\.sock"#;
const PORT_RE: &str =
    r"(?P<prefix>(?:127\.0\.0\.1|0\.0\.0\.0|localhost|\[::1\]):)(?P<port>\d{2,5})\b";
const TEMP_RE: &str = r#"(?:/private/tmp|/var/folders|/var/tmp|/tmp)[^\s"'<>]*"#;

struct Patterns {
    ulid: Regex,
    timestamp: Regex,
    socket: Regex,
    port: Regex,
    temp: Regex,
    qitem: Regex,
}

#[derive(Default)]
struct Masks {
    ulid: BTreeMap<String, String>,
    qitem: BTreeMap<String, String>,
    timestamp: BTreeMap<String, String>,
    path: BTreeMap<String, String>,
    port: BTreeMap<String, String>,
    socket: BTreeMap<String, String>,
    buffer: BTreeMap<String, String>,
}

impl Masks {
    fn assign(map: &mut BTreeMap<String, String>, prefix: &str, value: &str) -> String {
        if let Some(existing) = map.get(value) {
            return existing.clone();
        }
        let placeholder = format!("<{prefix}:{}>", map.len().saturating_add(1));
        map.insert(value.to_owned(), placeholder.clone());
        placeholder
    }
}

/// Apply `rules` to every text field. The same raw value keeps one placeholder.
///
/// # Errors
///
/// Returns [`Error::Pattern`] when a built-in expression cannot be compiled.
pub fn normalize_observation(
    obs: &Observation,
    rules: &[NormRule],
    roots: &[PathBuf],
) -> Result<Observation, Error> {
    let patterns = Patterns::compile()?;
    let mut masks = Masks::default();
    let mut tmux = obs.tmux_argv.clone();
    for argv in &mut tmux {
        mask_buffer_names(argv, &mut masks);
    }
    let stdout = apply_all(&obs.stdout, rules, roots, &patterns, &mut masks);
    let stderr = apply_all(&obs.stderr, rules, roots, &patterns, &mut masks);
    let json = match &obs.json {
        Some(value) => Some(norm_json(value, rules, roots, &patterns, &mut masks)?),
        None => None,
    };
    let mut files = BTreeMap::new();
    for (path, body) in &obs.files {
        let key = apply_all(
            &path.display().to_string(),
            rules,
            roots,
            &patterns,
            &mut masks,
        );
        let text = apply_all(body, rules, roots, &patterns, &mut masks);
        files.insert(PathBuf::from(key), text);
    }
    let mut tmux_argv = Vec::new();
    for argv in tmux {
        let mut next = Vec::new();
        for arg in argv {
            next.push(apply_all(&arg, rules, roots, &patterns, &mut masks));
        }
        tmux_argv.push(next);
    }
    Ok(Observation {
        stdout,
        stderr,
        exit: obs.exit,
        json,
        files,
        tmux_argv,
    })
}

/// Normalise one string. Used by the per-rule unit tests.
///
/// # Errors
///
/// Returns [`Error::Pattern`] when a built-in expression cannot be compiled.
pub fn normalize_text(text: &str, rules: &[NormRule], roots: &[PathBuf]) -> Result<String, Error> {
    let patterns = Patterns::compile()?;
    Ok(apply_all(
        text,
        rules,
        roots,
        &patterns,
        &mut Masks::default(),
    ))
}

fn mask_buffer_names(argv: &mut [String], masks: &mut Masks) {
    let mut previous_is_flag = false;
    for arg in argv.iter_mut() {
        if previous_is_flag {
            let placeholder = Masks::assign(&mut masks.buffer, "BUF", arg);
            *arg = placeholder;
            previous_is_flag = false;
            continue;
        }
        previous_is_flag = arg == "-b" || arg == "--buffer";
    }
}

fn norm_json(
    value: &serde_json::Value,
    rules: &[NormRule],
    roots: &[PathBuf],
    patterns: &Patterns,
    masks: &mut Masks,
) -> Result<serde_json::Value, Error> {
    match value {
        serde_json::Value::String(text) => Ok(serde_json::Value::String(apply_all(
            text, rules, roots, patterns, masks,
        ))),
        serde_json::Value::Array(items) => {
            let mut out = Vec::new();
            for item in items {
                out.push(norm_json(item, rules, roots, patterns, masks)?);
            }
            Ok(serde_json::Value::Array(out))
        }
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, item) in map {
                let next = norm_json(item, rules, roots, patterns, masks)?;
                out.insert(key.clone(), next);
            }
            Ok(serde_json::Value::Object(out))
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            Ok(value.clone())
        }
    }
}

fn strip_dash_getcwd(text: &str) -> String {
    const RAW: &str = "sh: 0: getcwd() failed: No such file or directory\n";
    const ESCAPED: &str = "sh: 0: getcwd() failed: No such file or directory\\n";
    text.replace(RAW, "").replace(ESCAPED, "")
}

fn apply_all(
    text: &str,
    rules: &[NormRule],
    roots: &[PathBuf],
    patterns: &Patterns,
    masks: &mut Masks,
) -> String {
    let mut current = text.to_owned();
    for rule in rules {
        current = apply_one(&current, rule, roots, patterns, masks);
    }
    current
}

fn apply_one(
    text: &str,
    rule: &NormRule,
    roots: &[PathBuf],
    patterns: &Patterns,
    masks: &mut Masks,
) -> String {
    match rule {
        NormRule::Ulid => replace_ulid(&patterns.ulid, text, &mut masks.ulid),
        NormRule::Timestamp => replace_whole(&patterns.timestamp, text, &mut masks.timestamp, "TS"),
        NormRule::SocketPath => replace_whole(&patterns.socket, text, &mut masks.socket, "SOCK"),
        NormRule::Port => replace_ports(&patterns.port, text, masks),
        NormRule::Getcwd => strip_dash_getcwd(text),
        NormRule::JsonCanon => json_canon(text),
        NormRule::Qitem => replace_whole(&patterns.qitem, text, &mut masks.qitem, "QITEM"),
        NormRule::AbsPath => {
            let with_roots = replace_roots(text, roots, masks);
            replace_whole(&patterns.temp, &with_roots, &mut masks.path, "PATH")
        }
        NormRule::Custom(regex, replacement) => regex
            .replace_all(text, |_caps: &Captures<'_>| (*replacement).to_owned())
            .into_owned(),
    }
}

fn replace_ulid(regex: &Regex, text: &str, map: &mut BTreeMap<String, String>) -> String {
    regex
        .replace_all(text, |caps: &Captures<'_>| {
            let Some(matched) = caps.get(0) else {
                return String::new();
            };
            let raw = matched.as_str();
            if ulid_bounded(text, matched.start(), matched.end()) {
                Masks::assign(map, "ULID", raw)
            } else {
                raw.to_owned()
            }
        })
        .into_owned()
}

fn ulid_bounded(text: &str, start: usize, end: usize) -> bool {
    let before = text
        .get(..start)
        .and_then(|prefix| prefix.chars().next_back());
    let after = text.get(end..).and_then(|suffix| suffix.chars().next());
    before.is_none_or(|ch| !is_ulid_char(ch)) && after.is_none_or(|ch| !is_ulid_char(ch))
}

fn is_ulid_char(ch: char) -> bool {
    matches!(
        ch.to_ascii_uppercase(),
        '0'..='9' | 'A'..='H' | 'J' | 'K' | 'M' | 'N' | 'P'..='T' | 'V'..='Z'
    )
}

fn replace_whole(
    regex: &Regex,
    text: &str,
    map: &mut BTreeMap<String, String>,
    prefix: &str,
) -> String {
    regex
        .replace_all(text, |caps: &Captures<'_>| {
            let raw = caps.get(0).map_or("", |matched| matched.as_str());
            Masks::assign(map, prefix, raw)
        })
        .into_owned()
}

fn replace_ports(regex: &Regex, text: &str, masks: &mut Masks) -> String {
    regex
        .replace_all(text, |caps: &Captures<'_>| {
            let prefix = caps.name("prefix").map_or("", |matched| matched.as_str());
            let port = caps.name("port").map_or("", |matched| matched.as_str());
            let placeholder = Masks::assign(&mut masks.port, "PORT", port);
            format!("{prefix}{placeholder}")
        })
        .into_owned()
}

fn replace_roots(text: &str, roots: &[PathBuf], masks: &mut Masks) -> String {
    let mut ordered: Vec<String> = roots
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    ordered.sort_by_key(|path| std::cmp::Reverse(path.len()));
    let mut current = text.to_owned();
    for root in &ordered {
        if root.is_empty() {
            continue;
        }
        current = replace_literal(&current, root, masks);
    }
    current
}

fn replace_literal(text: &str, needle: &str, masks: &mut Masks) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(index) = rest.find(needle) {
        let Some(before) = rest.get(..index) else {
            break;
        };
        out.push_str(before);
        let after = index.saturating_add(needle.len());
        let tail = rest.get(after..).unwrap_or("");
        let extra = tail
            .find(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | '<' | '>'))
            .unwrap_or(tail.len());
        let end = after.saturating_add(extra);
        let full = rest.get(index..end).unwrap_or(needle);
        out.push_str(&Masks::assign(&mut masks.path, "PATH", full));
        rest = rest.get(end..).unwrap_or("");
    }
    out.push_str(rest);
    out
}

impl Patterns {
    fn compile() -> Result<Self, Error> {
        Ok(Self {
            ulid: compile_re(ULID_RE)?,
            timestamp: compile_re(TIMESTAMP_RE)?,
            socket: compile_re(SOCKET_RE)?,
            port: compile_re(PORT_RE)?,
            temp: compile_re(TEMP_RE)?,
            qitem: compile_re(r"(?i)qitem-(?:[0-9A-HJKMNP-TV-Z]{26}|\d{14}-[0-9a-f]{8})")?,
        })
    }
}

fn json_canon(text: &str) -> String {
    let mut out = String::new();
    for line in text.split_inclusive('\n') {
        let newline = line.ends_with('\n');
        let trimmed = line.trim_end_matches('\n');
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed)
            && let Ok(encoded) = serde_json::to_string(&value)
        {
            out.push_str(&encoded);
            if newline {
                out.push('\n');
            }
            continue;
        }
        out.push_str(line);
    }
    out
}

fn compile_re(pattern: &str) -> Result<Regex, Error> {
    Regex::new(pattern).map_err(|err| Error::Pattern(err.to_string()))
}
