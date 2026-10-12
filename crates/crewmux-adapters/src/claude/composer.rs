//! Composer region. The current input is the last `❯` that a rule closes.

use crate::claude::staged::staged_matches;

/// A `❯` line that has a box-drawing rule after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ComposerAnchor {
    /// Line index of the prompt.
    pub(crate) line: usize,
    /// Text after the marker. An idle hint is `Try "…"`.
    pub(crate) body: String,
}

/// Last composer prompt that a `─` or `═` rule closes.
///
/// A numbered `❯ 1.` is a menu, not the composer. An ASCII `-` rule is not a
/// composer border: readiness stays on the box-drawing rules from real panes.
#[must_use]
pub(crate) fn composer_anchor(text: &str) -> Option<ComposerAnchor> {
    let lines: Vec<&str> = text.lines().collect();
    let mut found = None;
    for (index, line) in lines.iter().enumerate() {
        if !is_composer_prompt(line) {
            continue;
        }
        let closed = lines
            .iter()
            .skip(index.saturating_add(1))
            .any(|later| is_box_rule(later));
        if closed {
            found = Some(index);
        }
    }
    let index = found?;
    let body = prompt_body(lines.get(index)?);
    Some(ComposerAnchor { line: index, body })
}

/// Suffix that starts at the composer. Dialogs above that line are history.
#[must_use]
pub(crate) fn composer_view<'a>(text: &'a str, anchor: &ComposerAnchor) -> &'a str {
    let mut offset = 0usize;
    for (index, line) in text.lines().enumerate() {
        if index == anchor.line {
            return text.get(offset..).unwrap_or(text);
        }
        offset = offset.saturating_add(line.len()).saturating_add(1);
    }
    text
}

/// Empty prompt, or Claude's single-line `Try "…"` ghost hint.
#[must_use]
pub(crate) fn is_idle_body(body: &str) -> bool {
    let trimmed = body.trim().trim_matches('\u{a0}');
    if trimmed.is_empty() {
        return true;
    }
    let Some(rest) = trimmed.strip_prefix("Try \"") else {
        return false;
    };
    rest.ends_with('"') && !rest.chars().rev().skip(1).any(|ch| ch == '"')
}

/// The composer shows `payload` (boxed, inline, or a paste placeholder).
#[must_use]
pub(crate) fn composer_matches(screen: &str, payload: &str) -> bool {
    if let Some(body) = box_body(screen) {
        let body = squash(&strip_marker(&body));
        let raw = squash(payload);
        let wrapped = squash(&claude_visible(payload));
        if !body.is_empty() && (body == raw || body == wrapped) {
            return true;
        }
    }
    staged_matches(screen, payload)
}

/// Multi-line pastes are wrapped. Tabs become four spaces. Single-line text stays.
#[must_use]
pub(crate) fn claude_visible(payload: &str) -> String {
    let expanded = payload.replace('\t', "    ");
    if expanded.contains('\n') {
        format!("<pasted_content>\n{expanded}\n</pasted_content>")
    } else {
        expanded
    }
}

/// Collapse whitespace so a wrapped box still matches the payload.
#[must_use]
pub(crate) fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Boxed composer body, when the capture uses `╭` / `╰`.
#[must_use]
pub(crate) fn boxed_body(text: &str) -> Option<String> {
    box_body(text).map(|body| strip_marker(&body))
}

fn is_composer_prompt(line: &str) -> bool {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('❯') {
        return false;
    }
    let rest = trimmed
        .trim_start_matches('❯')
        .trim_start()
        .trim_start_matches('\u{a0}');
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return true;
    }
    !rest
        .get(digits.len()..)
        .is_some_and(|after| after.starts_with('.') || after.starts_with(". "))
}

fn is_box_rule(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.chars().count() >= 10 && trimmed.chars().all(|ch| ch == '─' || ch == '═')
}

fn prompt_body(line: &str) -> String {
    line.trim_start()
        .trim_start_matches('❯')
        .trim()
        .trim_matches('\u{a0}')
        .to_owned()
}

fn box_body(text: &str) -> Option<String> {
    let mut inside = false;
    let mut rows = Vec::new();
    for line in text.lines() {
        if line.contains('╭') {
            inside = true;
            continue;
        }
        if inside && line.contains('╰') {
            break;
        }
        if inside && let Some(inner) = between_bars(line) {
            rows.push(inner);
        }
    }
    if rows.is_empty() {
        None
    } else {
        Some(rows.join("\n"))
    }
}

fn between_bars(line: &str) -> Option<String> {
    let start = line.find('│')?;
    let end = line.rfind('│')?;
    if end <= start {
        return None;
    }
    let inner = line.get(start.saturating_add('│'.len_utf8())..end)?;
    Some(inner.trim().to_owned())
}

fn strip_marker(body: &str) -> String {
    let mut lines = body.lines();
    let Some(first) = lines.next() else {
        return String::new();
    };
    let first = first
        .trim()
        .trim_start_matches(['❯', '>', '›'])
        .trim()
        .trim_matches('\u{a0}');
    let mut out = String::from(first);
    for line in lines {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line.trim());
    }
    out
}
