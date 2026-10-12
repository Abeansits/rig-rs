//! Option menus shared by the Claude classifier and the dialog driver.
//!
//! A bare `❯` prompt is the composer, not a choice. A single unnumbered cursor
//! line is conversation text. Two choices, or any numbered choice, is a menu.

/// One choice parsed from a capture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuLine {
    /// Text after the cursor marker, including a leading `1.` when present.
    pub text: String,
    /// Number parsed from a `N.` prefix.
    pub index: Option<u8>,
    /// The line carried `❯` or `›`.
    pub cursor: bool,
}

/// Choice lines in screen order.
///
/// An indented line just before a cursor or numbered choice is part of the
/// same menu. That is how "No, exit" stays visible after Down moves onto
/// "Yes, I trust this folder".
#[must_use]
pub fn menu_lines(text: &str) -> Vec<MenuLine> {
    let mut lines = Vec::new();
    let mut leading = Vec::new();
    for line in text.lines() {
        if let Some(option) = as_option(line) {
            lines.append(&mut leading);
            lines.push(option);
            continue;
        }
        if is_sibling(line) {
            let sibling = sibling_line(line);
            if lines.last().is_some() {
                lines.push(sibling);
            } else {
                leading.push(sibling);
            }
            continue;
        }
        leading.clear();
    }
    lines
}

fn sibling_line(line: &str) -> MenuLine {
    MenuLine {
        text: line.trim().to_owned(),
        index: None,
        cursor: false,
    }
}

/// A menu is two or more choices, or any numbered choice.
#[must_use]
pub fn is_option_menu(lines: &[MenuLine]) -> bool {
    lines.len() >= 2 || lines.iter().any(|line| line.index.is_some())
}

/// Index of the cursor choice.
#[must_use]
pub fn cursor_index(lines: &[MenuLine]) -> Option<usize> {
    lines.iter().position(|line| line.cursor)
}

fn as_option(line: &str) -> Option<MenuLine> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (cursor, rest) = strip_cursor(trimmed);
    if rest.is_empty() {
        return None;
    }
    let index = leading_index(rest);
    if !cursor && index.is_none() {
        return None;
    }
    Some(MenuLine {
        text: rest.to_owned(),
        index,
        cursor,
    })
}

fn strip_cursor(trimmed: &str) -> (bool, &str) {
    if let Some(rest) = trimmed.strip_prefix('❯') {
        return (true, rest.trim());
    }
    if let Some(rest) = trimmed.strip_prefix('›') {
        return (true, rest.trim());
    }
    (false, trimmed)
}

fn leading_index(rest: &str) -> Option<u8> {
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let after = rest.get(digits.len()..)?;
    if after.starts_with(". ") || after == "." {
        digits.parse().ok()
    } else {
        None
    }
}

fn is_sibling(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.contains('─') || trimmed.contains("for shortcuts") {
        return false;
    }
    line.starts_with(' ') || line.starts_with('\t')
}
