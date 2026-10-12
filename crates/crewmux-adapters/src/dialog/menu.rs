//! Option menus shared by the classifier and the dialog driver.
//!
//! A bare `❯` prompt is the composer, not a choice. A single unnumbered cursor
//! line is conversation text. Two choices, or any numbered choice, is a menu.
//! An indented sibling stays in the menu after the cursor moves off it.

/// One choice parsed from a capture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuOption {
    /// Label with a leading `N.` removed.
    pub label: String,
    /// Number parsed from a `N.` prefix.
    pub number: Option<u8>,
    /// The line carried `❯` or `›`.
    pub selected: bool,
    /// Line index in the capture.
    pub line: usize,
}

/// Choice lines in screen order.
///
/// An indented line just before a cursor or numbered choice is part of the
/// same menu. That is how "No, exit" stays visible after Down moves onto
/// "Yes, I trust this folder".
#[must_use]
pub fn parse_menu(text: &str) -> Vec<MenuOption> {
    let mut lines = Vec::new();
    let mut leading = Vec::new();
    let mut after_blank_or_choice = true;
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            leading.clear();
            after_blank_or_choice = true;
            continue;
        }
        if let Some(option) = as_option(line, index) {
            lines.append(&mut leading);
            lines.push(option);
            after_blank_or_choice = true;
            continue;
        }
        if after_blank_or_choice && is_numbered_choice(line) {
            lines.append(&mut leading);
            lines.push(sibling_line(line, index));
            after_blank_or_choice = true;
            continue;
        }
        if is_sibling(line) {
            let sibling = sibling_line(line, index);
            if lines.last().is_some() {
                lines.push(sibling);
            } else {
                leading.push(sibling);
            }
            after_blank_or_choice = false;
            continue;
        }
        leading.clear();
        after_blank_or_choice = false;
    }
    lines
}

/// A menu is two or more choices, or any numbered choice.
#[must_use]
pub fn is_option_menu(lines: &[MenuOption]) -> bool {
    lines.len() >= 2 || lines.iter().any(|line| line.number.is_some())
}

fn sibling_line(line: &str, index: usize) -> MenuOption {
    let text = line.trim();
    let (number, label) = split_label(text);
    MenuOption {
        label,
        number,
        selected: false,
        line: index,
    }
}

fn is_numbered_choice(line: &str) -> bool {
    let trimmed = line.trim();
    let (selected, rest) = strip_cursor(trimmed);
    if selected || rest.is_empty() {
        return false;
    }
    let (number, label) = split_label(rest);
    number.is_some() && !label.is_empty()
}

fn as_option(line: &str, index: usize) -> Option<MenuOption> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (selected, rest) = strip_cursor(trimmed);
    if rest.is_empty() {
        return None;
    }
    let (number, label) = split_label(rest);
    // A bare `5. …` line is wrapped prose (`Sonnet` / `5. Switch…`), not a choice.
    // Numbered choices without the cursor join as siblings of a cursor line.
    if !selected {
        return None;
    }
    // A column-0 unnumbered `›` is the Codex composer (`› Ask Codex…`), not a choice.
    // `❯ Resume from summary` and indented ` ❯ No, exit` stay choices.
    if number.is_none() && line.starts_with('›') {
        return None;
    }
    if label.is_empty() {
        return None;
    }
    Some(MenuOption {
        label,
        number,
        selected,
        line: index,
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

fn split_label(rest: &str) -> (Option<u8>, String) {
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return (None, choice_name(rest));
    }
    let Some(after) = rest.get(digits.len()..) else {
        return (None, rest.to_owned());
    };
    if after.starts_with(". ") || after == "." {
        let label = choice_name(after.trim_start_matches('.').trim());
        (digits.parse().ok(), label)
    } else {
        (None, choice_name(rest))
    }
}

/// Help text after a Codex choice is separated by two or more spaces.
fn choice_name(label: &str) -> String {
    match label.find("  ") {
        Some(index) => label.get(..index).unwrap_or(label).trim().to_owned(),
        None => label.to_owned(),
    }
}

fn is_sibling(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty()
        || trimmed.contains('─')
        || trimmed.contains("for shortcuts")
        || trimmed.contains("Enter to confirm")
        || trimmed.contains("Esc to cancel")
        || trimmed.contains("to navigate")
        || trimmed.contains("enter continue")
        || trimmed.contains("enter to submit")
        || trimmed.to_ascii_lowercase().contains("esc to cancel")
        || trimmed.contains('⏵')
        || trimmed.contains('⏸')
        || trimmed.contains("shift+tab")
        || trimmed.contains("for agents")
        || trimmed.contains("paste again")
    {
        return false;
    }
    line.starts_with(' ') || line.starts_with('\t')
}
