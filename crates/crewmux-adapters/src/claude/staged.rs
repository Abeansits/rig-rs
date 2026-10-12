//! On-screen paste check.
//!
//! Derived from openrig packages/daemon/src/domain/session-transport.ts
//! `hasExpectedStagedText` (v0.6.9, Apache-2.0); ported to Rust and modified.
//! A single `[Pasted text #N +X lines]` token counts only when N is a positive
//! integer and X equals the payload's newline count (`matches('\n')`, not
//! `split('\n')`). A 24-character prefix of a longer payload is not a match.
//! Several tokens still need the residual suffix to land on an exact
//! source-line boundary.

/// The composer shows `payload`, or Claude's collapsed paste of it.
#[must_use]
pub(crate) fn staged_matches(screen: &str, payload: &str) -> bool {
    let Some(region) = input_region(screen) else {
        return false;
    };
    if payload.is_empty() {
        return false;
    }
    let flat = collapse_ws(&region);
    let counts = placeholder_counts(&flat);
    if counts.is_empty() {
        let piece = squash_ws(payload);
        return !piece.is_empty() && squash_ws(&region).contains(&piece);
    }
    if counts.len() == 1
        && let Some(count) = counts.first().copied()
        && count > 0
        && count == line_count(payload)
    {
        return true;
    }
    let residual = residual_text(&flat);
    let piece = squash_ws(payload);
    if residual.chars().count() < 48 || !piece.ends_with(&residual) {
        return false;
    }
    let Some(boundary) = source_boundary(payload, residual.chars().count()) else {
        return false;
    };
    let sum: usize = counts.iter().copied().sum();
    sum == boundary
}

fn input_region(screen: &str) -> Option<String> {
    let lines: Vec<&str> = screen.lines().collect();
    let mut current = None;
    for (index, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with('❯') {
            current = Some(index);
        }
    }
    let start = current?;
    let input = lines.get(start)?.trim_start();
    if numbered_choice(input) {
        return None;
    }
    let mut end = lines.len();
    for (index, line) in lines.iter().enumerate().skip(start.saturating_add(1)) {
        if is_rule(line) {
            end = index;
            break;
        }
    }
    Some(lines.get(start..end)?.join("\n"))
}

fn numbered_choice(input: &str) -> bool {
    let rest = input.trim_start_matches('❯').trim_start();
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return false;
    }
    rest.get(digits.len()..)
        .is_some_and(|after| after.starts_with('.') || after.starts_with(". "))
}

fn is_rule(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.chars().count() >= 10
        && trimmed
            .chars()
            .all(|ch| ch == '─' || ch == '═' || ch == '-')
}

fn line_count(payload: &str) -> usize {
    payload.matches('\n').count()
}

fn collapse_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn squash_ws(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn placeholder_counts(flat: &str) -> Vec<usize> {
    let mut counts = Vec::new();
    let mut rest = flat;
    let marker = "[Pasted text #";
    while let Some(start) = rest.find(marker) {
        let Some(after) = rest.get(start.saturating_add(marker.len())..) else {
            break;
        };
        match pasted_count(after) {
            Some((count, next)) => {
                counts.push(count);
                rest = next;
            }
            None => rest = after.get(1..).unwrap_or(""),
        }
    }
    counts
}

/// `#N +X lines]` when N is a positive integer. `next` starts after the token.
fn pasted_count(after: &str) -> Option<(usize, &str)> {
    let id_len = after.chars().take_while(char::is_ascii_digit).count();
    if id_len == 0 {
        return None;
    }
    let id = after.get(..id_len)?;
    id.parse::<u64>().ok().filter(|value| *value >= 1)?;
    let tail = after.get(id_len..)?.strip_prefix(" +")?;
    let count_len = tail.chars().take_while(char::is_ascii_digit).count();
    if count_len == 0 {
        return None;
    }
    let digits = tail.get(..count_len)?;
    let suffix = tail.get(count_len..)?;
    let next = suffix.strip_prefix(" lines]")?;
    let count = digits.parse().ok()?;
    Some((count, next))
}

fn residual_text(flat: &str) -> String {
    let mut kept = String::new();
    let mut rest = flat;
    while !rest.is_empty() {
        if let Some(stripped) = strip_prefix_ci(rest, "[Pasted text #")
            && let Some(end) = stripped.find(" lines]")
        {
            rest = stripped
                .get(end.saturating_add(" lines]".len())..)
                .unwrap_or("");
            continue;
        }
        if let Some(stripped) = strip_prefix_ci(rest, "paste again to expand") {
            rest = stripped;
            continue;
        }
        if let Some(stripped) = strip_prefix_ci(rest, "ctrl+g to edit in vim") {
            rest = stripped;
            continue;
        }
        if let Some(stripped) = strip_prefix_ci(rest, "ctrl+g to edit") {
            rest = stripped;
            continue;
        }
        let Some(ch) = rest.chars().next() else {
            break;
        };
        kept.push(ch);
        rest = rest.get(ch.len_utf8()..).unwrap_or("");
    }
    let squashed = squash_ws(&kept);
    squashed.strip_prefix('❯').unwrap_or(&squashed).to_owned()
}

fn strip_prefix_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let mut chars = text.chars();
    for expected in prefix.chars() {
        let found = chars.next()?;
        if !found.eq_ignore_ascii_case(&expected) {
            return None;
        }
    }
    Some(chars.as_str())
}

fn source_boundary(payload: &str, residual_chars: usize) -> Option<usize> {
    let lines: Vec<&str> = payload.split('\n').collect();
    let mut acc = 0usize;
    for index in (0..lines.len()).rev() {
        let line = lines.get(index)?;
        acc = acc.saturating_add(squash_ws(line).chars().count());
        if acc >= residual_chars {
            return Some(index);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::staged_matches;

    #[test]
    fn single_placeholder_requires_the_payload_line_count() {
        let screen = "\
────────────────────────────────────────────────────────────────────────────────
❯ [Pasted text #2 +29 lines]
────────────────────────────────────────────────────────────────────────────────
  paste again to expand
";
        let payload = "alpha line of pasted text\n".repeat(29);
        assert_eq!(payload.matches('\n').count(), 29);
        assert_eq!(
            payload.split('\n').count(),
            30,
            "split counts a trailing newline as an extra line"
        );
        assert!(staged_matches(screen, &payload), "29 newlines");
        let short = "alpha line of pasted text\n".repeat(28) + "alpha line of pasted text";
        assert_eq!(short.matches('\n').count(), 28);
        assert!(!staged_matches(screen, &short), "28 newlines is not +29");
        let nearby = format!("{payload}extra");
        assert_eq!(nearby.matches('\n').count(), 29);
        assert!(
            staged_matches(screen, &nearby),
            "the count is newlines, not the last line's text"
        );
        let extra_newline = format!("{payload}\nextra");
        assert_eq!(extra_newline.matches('\n').count(), 30);
        assert!(
            !staged_matches(screen, &extra_newline),
            "30 newlines is not this placeholder"
        );
        assert!(
            !staged_matches(screen, "hello"),
            "an unrelated placeholder is not a match"
        );
        let one = "\
────────────────────────────────────────────────────────────────────────────────
❯ [Pasted text #1 +2 lines]
────────────────────────────────────────────────────────────────────────────────
";
        assert!(!staged_matches(one, "hello"), "+2 is not zero newlines");
        assert!(!staged_matches(one, "hello\n"), "+2 is not one newline");
        let bare = "\
────────────────────────────────────────────────────────────────────────────────
❯ [Pasted text # +1 lines]
────────────────────────────────────────────────────────────────────────────────
";
        assert!(!staged_matches(bare, "hello"), "a placeholder needs #N");
        let named = "\
────────────────────────────────────────────────────────────────────────────────
❯ [Pasted text #1 +1 lines]
────────────────────────────────────────────────────────────────────────────────
";
        assert!(
            !staged_matches(named, "hello"),
            "a 1-line payload with no newline is +0"
        );
        assert!(staged_matches(named, "hello\n"));
        let long = "abcdefghijklmnopqrstuvwxyz-more-than-twenty-four";
        let partial = format!(
            "\
────────────────────────────────────────────────────────────────────────────────
❯ {}
────────────────────────────────────────────────────────────────────────────────
",
            long.chars().take(24).collect::<String>()
        );
        assert!(
            !staged_matches(&partial, long),
            "a 24-character prefix is not the payload"
        );
        let full = format!(
            "\
────────────────────────────────────────────────────────────────────────────────
❯ {long}
────────────────────────────────────────────────────────────────────────────────
"
        );
        assert!(staged_matches(&full, long));
    }

    #[test]
    fn collapsed_capture_matches_a_29_line_payload() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/screens/claude/2.1.289/collapsed-paste-hint.txt");
        let screen = std::fs::read_to_string(&path).expect("fixture");
        let payload = "line of pasted text\n".repeat(29);
        assert_eq!(payload.matches('\n').count(), 29);
        assert!(staged_matches(&screen, &payload), "{screen}");
        let short = "line of pasted text\n".repeat(28) + "line of pasted text";
        assert_eq!(short.matches('\n').count(), 28);
        assert!(!staged_matches(&screen, &short), "28 newlines is not +29");
        assert!(!staged_matches(&screen, "not the paste"));
    }

    #[test]
    fn collapsed_label_counts_newlines_not_split_lines() {
        let prompt = (0..96)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(prompt.matches('\n').count(), 95);
        assert_eq!(prompt.split('\n').count(), 96);
        assert!(staged_matches(&composer(95), &prompt), "+95 for 96 lines");
        assert!(
            !staged_matches(&composer(96), &prompt),
            "+96 is the split count, not Claude's newline count"
        );
        let file = format!(
            "{prompt_29}\n",
            prompt_29 = (0..29)
                .map(|index| format!("line {index}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        assert_eq!(file.matches('\n').count(), 29);
        assert!(
            staged_matches(&composer(29), &file),
            "a trailing newline is part of the count"
        );
        assert!(!staged_matches(&composer(28), &file));
    }

    fn composer(newlines: usize) -> String {
        format!(
            "\
────────────────────────────────────────────────────────────────────────────────
❯ [Pasted text #1 +{newlines} lines]
────────────────────────────────────────────────────────────────────────────────
  paste again to expand
"
        )
    }
}
