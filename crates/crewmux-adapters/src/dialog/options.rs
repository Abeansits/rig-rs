//! Numbered and cursor-marked option lines.

/// One option row on a provider dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuOption {
    /// Text after the cursor and the `N.` prefix.
    pub label: String,
    /// The line carries `›` or `❯`.
    pub selected: bool,
    /// Zero-based line index in the capture.
    pub line: usize,
}

/// Option rows in a capture. A composer box is not an option.
#[must_use]
pub fn menu_options(text: &str) -> Vec<MenuOption> {
    let mut out = Vec::new();
    for (line, row) in text.lines().enumerate() {
        if let Some((selected, label)) = option_label(row) {
            out.push(MenuOption {
                label,
                selected,
                line,
            });
        }
    }
    out
}

fn option_label(line: &str) -> Option<(bool, String)> {
    let trimmed = line.trim_end();
    if trimmed.trim().is_empty() {
        return None;
    }
    let body = trimmed.trim_start();
    let (selected, rest) = strip_cursor(body);
    if rest.is_empty() || is_chrome(rest) {
        return None;
    }
    let label = strip_number(rest).unwrap_or_else(|| rest.to_owned());
    if label.is_empty() || is_chrome(&label) {
        return None;
    }
    if selected || strip_number(rest).is_some() {
        Some((selected, label))
    } else {
        None
    }
}

fn strip_cursor(text: &str) -> (bool, &str) {
    let mut chars = text.chars();
    match chars.next() {
        Some('›' | '❯') => (true, chars.as_str().trim_start()),
        _ => (false, text),
    }
}

fn strip_number(text: &str) -> Option<String> {
    let mut digits = 0usize;
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            digits = digits.saturating_add(1);
        } else {
            break;
        }
    }
    if digits == 0 {
        return None;
    }
    let rest = text.get(digits..)?;
    let rest = rest.strip_prefix('.')?.trim_start();
    if rest.is_empty() {
        None
    } else {
        Some(rest.to_owned())
    }
}

fn is_chrome(text: &str) -> bool {
    text.chars().all(|ch| {
        matches!(
            ch,
            '╭' | '╮' | '╰' | '╯' | '│' | '─' | ' ' | '┌' | '┐' | '└' | '┘'
        )
    })
}
