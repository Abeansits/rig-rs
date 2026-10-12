//! How a seat shows a paste, and how `rig ask` finds that text in a transcript.
//!
//! A real Claude Code login wraps a multi-line paste in `<pasted_content>` and
//! turns each tab into four spaces. Codex leaves the bytes unchanged. Stub and
//! every other runtime do too.

use crewmux_types::RuntimeKind;

/// Which paste shape a seat's pane and transcript use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PasteDisplay {
    /// The pasted bytes. Codex, the stub, and every runtime except Claude.
    #[default]
    Exact,
    /// Claude Code's composer and pane transcript.
    Claude,
}

impl PasteDisplay {
    /// Display for a seat runtime. Only Claude rewrites the paste.
    #[must_use]
    pub const fn for_runtime(runtime: RuntimeKind) -> Self {
        match runtime {
            RuntimeKind::ClaudeCode => Self::Claude,
            RuntimeKind::Codex
            | RuntimeKind::OpenCode
            | RuntimeKind::Terminal
            | RuntimeKind::Stub => Self::Exact,
        }
    }

    /// `true` when this display may rewrite tabs and wrap multi-line pastes.
    #[must_use]
    pub const fn is_claude(self) -> bool {
        matches!(self, Self::Claude)
    }
}

/// Tabs become four spaces. Other bytes stay.
#[must_use]
pub fn expand_tabs(text: &str) -> String {
    text.replace('\t', "    ")
}

/// Text Claude shows for `payload`.
///
/// A paste that contains a newline is wrapped in `<pasted_content>` on its own
/// lines. A single line is only tab-expanded.
#[must_use]
pub fn claude_visible(payload: &str) -> String {
    let expanded = expand_tabs(payload);
    if !expanded.contains('\n') {
        return expanded;
    }
    let mut out = String::from("<pasted_content>\n");
    out.push_str(&expanded);
    if !expanded.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("</pasted_content>");
    out
}

/// A transcript line or block that matches `needle`.
///
/// Exact displays require the needle's bytes on one line. Claude also matches
/// after tab expansion, including a multi-line needle inside a
/// `<pasted_content>` block.
#[must_use]
pub fn transcript_hit(display: PasteDisplay, text: &str, needle: &str) -> Option<String> {
    if needle.is_empty() {
        return None;
    }
    if let Some(line) = line_containing(text, needle) {
        return Some(line);
    }
    if !display.is_claude() {
        return None;
    }
    let needle = expand_tabs(needle);
    if let Some(line) = text
        .lines()
        .find(|line| expand_tabs(line).contains(&needle))
    {
        return Some(line.to_owned());
    }
    let expanded = expand_tabs(text);
    if let Some(found) = block_containing(&expanded, &needle) {
        return Some(found);
    }
    block_containing(&unwrap_pasted(&expanded), &needle)
}

fn line_containing(text: &str, needle: &str) -> Option<String> {
    text.lines()
        .find(|line| line.contains(needle))
        .map(str::to_owned)
}

fn block_containing(haystack: &str, needle: &str) -> Option<String> {
    if !needle.contains('\n') {
        return None;
    }
    let start = haystack.find(needle)?;
    haystack
        .get(start..start.saturating_add(needle.len()))
        .map(str::to_owned)
}

fn unwrap_pasted(text: &str) -> String {
    let mut out = String::new();
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed == "<pasted_content>" || trimmed == "</pasted_content>" {
            continue;
        }
        out.push_str(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{PasteDisplay, claude_visible, expand_tabs, transcript_hit};

    #[test]
    fn claude_wraps_a_multiline_paste_and_expands_tabs() {
        let visible = claude_visible("a\tb\nsecond");
        assert_eq!(
            visible,
            "<pasted_content>\na    b\nsecond\n</pasted_content>"
        );
        assert_eq!(claude_visible("a\tb"), "a    b");
        assert_eq!(expand_tabs("a\tb"), "a    b");
    }

    #[test]
    fn claude_transcript_matches_tabs_and_a_wrapped_block() {
        let text = "<pasted_content>\ncol    value\nnext\n</pasted_content>\n";
        let line = transcript_hit(PasteDisplay::Claude, text, "col\tvalue").expect("line");
        assert_eq!(line, "col    value");
        let block = transcript_hit(PasteDisplay::Claude, text, "col\tvalue\nnext").expect("block");
        assert_eq!(block, "col    value\nnext");
        assert!(transcript_hit(PasteDisplay::Exact, text, "col\tvalue").is_none());
        let exact = transcript_hit(PasteDisplay::Exact, "col\tvalue\n", "col\tvalue").expect("tab");
        assert_eq!(exact, "col\tvalue");
    }
}
