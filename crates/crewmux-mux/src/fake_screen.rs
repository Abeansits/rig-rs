//! Capture rewrites the in-memory multiplexer uses when a paste lands.

use crate::types::{Capture, CaptureRange};

pub(crate) fn land_or_append(screen: Capture, text: &str) -> Capture {
    if screen.text.contains('›') {
        let spliced = spliced_composer(&screen.text, text);
        if spliced != screen.text {
            return Capture::new(spliced, screen.cursor);
        }
    }
    let landed = land_paste(&screen, text);
    if landed.text != screen.text {
        return landed;
    }
    let mut next = screen.text;
    next.push_str(text);
    Capture::new(next, screen.cursor)
}

/// Put `payload` into an empty `>` / `❯` prompt so a paste check can see it.
///
/// A replay queue is left alone: the test is showing a screen that did not change.
fn land_paste(capture: &Capture, payload: &str) -> Capture {
    let flat = payload.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut lines: Vec<String> = capture.text.lines().map(str::to_owned).collect();
    if let Some(index) = lines.iter().rposition(|line| empty_prompt_line(line))
        && let Some(line) = lines.get_mut(index)
    {
        *line = fill_prompt(line, &flat);
    }
    let mut text = lines.join("\n");
    if capture.text.ends_with('\n') {
        text.push('\n');
    }
    Capture::new(text, capture.cursor)
}

fn empty_prompt_line(line: &str) -> bool {
    let inner = bar_inner(line).unwrap_or(line.trim());
    let Some(body) = strip_prompt_marker(inner) else {
        return false;
    };
    let body = body.trim().trim_matches('\u{a0}');
    body.is_empty() || is_idle_hint(body)
}

fn is_idle_hint(body: &str) -> bool {
    let Some(rest) = body.strip_prefix("Try \"") else {
        return false;
    };
    rest.ends_with('"') && !rest.chars().rev().skip(1).any(|ch| ch == '"')
}

fn strip_prompt_marker(text: &str) -> Option<&str> {
    text.trim()
        .strip_prefix('❯')
        .or_else(|| text.trim().strip_prefix('>'))
        .or_else(|| text.trim().strip_prefix('›'))
}

/// One `C-u`: drop the last continuation inside the composer, or blank the prompt.
///
/// Enter still uses [`clear_prompt`], which blanks the prompt line in one step.
pub(crate) fn clear_one_draft_line(capture: &Capture) -> Capture {
    let mut lines: Vec<String> = capture.text.lines().map(str::to_owned).collect();
    let Some(prompt) = lines.iter().rposition(|line| prompt_marker_line(line)) else {
        return clear_prompt(capture);
    };
    if let Some(index) = last_continuation(&lines, prompt) {
        lines.remove(index);
        return joined(capture, &lines);
    }
    if let Some(line) = lines.get_mut(prompt) {
        *line = fill_prompt(line, "");
    }
    joined(capture, &lines)
}

fn prompt_marker_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('❯') || trimmed.starts_with('›')
}

fn last_continuation(lines: &[String], prompt: usize) -> Option<usize> {
    let end = lines
        .iter()
        .enumerate()
        .skip(prompt.saturating_add(1))
        .find(|(_, line)| is_box_rule(line))
        .map_or(lines.len(), |(index, _)| index);
    (prompt.saturating_add(1)..end).rev().find(|index| {
        lines
            .get(*index)
            .is_some_and(|line| !line.trim().is_empty())
    })
}

fn is_box_rule(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.chars().count() >= 10 && trimmed.chars().all(|ch| ch == '─' || ch == '═')
}

fn joined(capture: &Capture, lines: &[String]) -> Capture {
    let mut text = lines.join("\n");
    if capture.text.ends_with('\n') {
        text.push('\n');
    }
    Capture::new(text, capture.cursor)
}

pub(crate) fn clear_prompt(capture: &Capture) -> Capture {
    let mut lines: Vec<String> = capture.text.lines().map(str::to_owned).collect();
    if let Some(index) = lines.iter().rposition(|line| {
        empty_prompt_line(line) || strip_prompt_marker(line).is_some() || line.contains('│')
    }) && let Some(line) = lines.get_mut(index)
        && (line.contains('❯') || line.contains('>') || line.contains('›'))
    {
        *line = fill_prompt(line, "");
    }
    let mut text = lines.join("\n");
    if capture.text.ends_with('\n') {
        text.push('\n');
    }
    Capture::new(text, capture.cursor)
}

fn fill_prompt(line: &str, flat: &str) -> String {
    let Some(start) = line.find('│') else {
        return rewrite_marker(line, flat);
    };
    let Some(end) = line.rfind('│') else {
        return format!("> {flat}");
    };
    if end <= start {
        return format!("> {flat}");
    }
    let pre = line.get(..start).unwrap_or("");
    let post = line.get(end..).unwrap_or("");
    let inside = line.get(start.saturating_add(1)..end).unwrap_or("");
    let marker = if inside.contains('❯') {
        "❯"
    } else if inside.contains('›') {
        "›"
    } else {
        ">"
    };
    if flat.is_empty() {
        format!("{pre}│ {marker} {post}")
    } else {
        format!("{pre}│ {marker} {flat} {post}")
    }
}

/// Insert `pasted` after the last `›`, keeping a closing box bar on the last line.
fn spliced_composer(capture: &str, pasted: &str) -> String {
    let Some(cursor) = capture.rfind('›') else {
        return capture.to_owned();
    };
    let Some(mark) = capture.get(cursor..).and_then(|rest| rest.chars().next()) else {
        return capture.to_owned();
    };
    let prefix_end = cursor.saturating_add(mark.len_utf8());
    let line_start = capture
        .get(..cursor)
        .and_then(|head| head.rfind('\n'))
        .map_or(0, |index| index.saturating_add(1));
    let line_end = capture
        .get(cursor..)
        .and_then(|rest| rest.find('\n'))
        .map_or(capture.len(), |index| cursor.saturating_add(index));
    let Some(prefix) = capture.get(line_start..prefix_end) else {
        return capture.to_owned();
    };
    let trailing = capture
        .get(prefix_end..line_end)
        .is_some_and(|tail| tail.trim_end().ends_with(['│', '┃']));
    let suffix = if trailing { " │" } else { "" };
    let mut rebuilt = String::new();
    rebuilt.push_str(capture.get(..line_start).unwrap_or(""));
    rebuilt.push_str(prefix);
    rebuilt.push(' ');
    rebuilt.push_str(pasted);
    rebuilt.push_str(suffix);
    rebuilt.push_str(capture.get(line_end..).unwrap_or(""));
    rebuilt
}

fn rewrite_marker(line: &str, flat: &str) -> String {
    let trimmed = line.trim_start();
    let indent_len = line.len().saturating_sub(trimmed.len());
    let indent = line.get(..indent_len).unwrap_or("");
    let marker = if trimmed.starts_with('❯') {
        "❯"
    } else if trimmed.starts_with('›') {
        "›"
    } else {
        ">"
    };
    if flat.is_empty() {
        format!("{indent}{marker}")
    } else {
        format!("{indent}{marker} {flat}")
    }
}

fn bar_inner(line: &str) -> Option<&str> {
    let start = line.find('│')?;
    let end = line.rfind('│')?;
    if end <= start {
        return None;
    }
    line.get(start.saturating_add('│'.len_utf8())..end)
        .map(str::trim)
}

pub(crate) fn slice_capture(capture: Capture, range: CaptureRange) -> Capture {
    let lines = match range {
        CaptureRange::Visible | CaptureRange::Joined => return capture,
        CaptureRange::LastLines(n) => usize::from(n),
        CaptureRange::History(n) | CaptureRange::JoinedHistory(n) => {
            usize::try_from(n).unwrap_or(usize::MAX)
        }
    };
    let trailing = capture.text.ends_with('\n');
    let kept: Vec<&str> = capture.text.lines().collect();
    let count = kept.len();
    let skip = count.saturating_sub(lines);
    let mut text = kept.into_iter().skip(skip).collect::<Vec<_>>().join("\n");
    // `lines` drops the terminator. An exact append is `before + payload`, so a
    // prompt that ended with a newline has to keep it or every paste looks partial.
    if trailing && skip < count {
        text.push('\n');
    }
    Capture::new(text, capture.cursor)
}

#[cfg(test)]
mod tests {
    use super::{Capture, clear_one_draft_line};

    #[test]
    fn ctrl_u_drops_one_typed_line_then_the_prompt() {
        let rule = "─".repeat(20);
        let text = format!("{rule}\n❯ From: operator@host\nTo: cl@pr18\n{rule}\n");
        let once = clear_one_draft_line(&Capture::new(text, (0, 0)));
        assert!(once.text.contains("From:"), "{}", once.text);
        assert!(!once.text.contains("To:"), "{}", once.text);
        let twice = clear_one_draft_line(&once);
        assert!(!twice.text.contains("From:"), "{}", twice.text);
        assert!(twice.text.contains('❯'), "{}", twice.text);
    }
}
