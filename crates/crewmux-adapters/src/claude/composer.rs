//! Composer box text. Ready is an empty prompt. A draft fails closed.

/// Text inside the input box, with the `>` / `❯` marker removed.
///
/// `None` when the capture has no composer box and no prompt line.
#[must_use]
pub(crate) fn composer_contents(text: &str) -> Option<String> {
    if let Some(body) = box_body(text) {
        return Some(strip_marker(&body));
    }
    prompt_line_body(text)
}

/// The box (or a bare prompt line) has text after the marker.
#[must_use]
pub(crate) fn is_draft(text: &str) -> bool {
    composer_contents(text).is_some_and(|body| !squash(&body).is_empty())
}

/// Empty prompt plus the shortcuts footer and a border.
#[must_use]
pub(crate) fn is_empty_composer(text: &str) -> bool {
    text.contains("? for shortcuts")
        && has_border(text)
        && composer_contents(text).is_some_and(|body| squash(&body).is_empty())
}

/// The composer shows `payload`, or Claude's on-screen wrap of it, and nothing else.
#[must_use]
pub(crate) fn composer_matches(screen: &str, payload: &str) -> bool {
    let Some(body) = composer_contents(screen) else {
        return false;
    };
    let body = squash(&body);
    if body.is_empty() || payload.is_empty() {
        return false;
    }
    let raw = squash(payload);
    let wrapped = squash(&claude_visible(payload));
    body == raw || body == wrapped
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

fn prompt_line_body(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = strip_one(trimmed) {
            return Some(rest.trim().to_owned());
        }
    }
    None
}

fn strip_marker(body: &str) -> String {
    let mut lines = body.lines();
    let Some(first) = lines.next() else {
        return String::new();
    };
    let first = strip_one(first.trim()).unwrap_or(first).trim();
    let mut out = String::from(first);
    for line in lines {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line.trim());
    }
    out
}

fn strip_one(trimmed: &str) -> Option<&str> {
    trimmed
        .strip_prefix('❯')
        .or_else(|| trimmed.strip_prefix('>'))
        .or_else(|| trimmed.strip_prefix('›'))
}

fn has_border(text: &str) -> bool {
    text.lines().any(|line| {
        line.contains('╭')
            || line.contains('╰')
            || line.chars().filter(|ch| *ch == '─').take(4).count() >= 4
    })
}
