//! Composer parsers. Claude and Codex land in later slices.
//!
//! The stub parser is the only one this slice ships. `ComposerParser::Stub`
//! is the seam those parsers plug into: a region, a [`ComposerState`], and the
//! keys that clear a failed paste.

use std::time::Duration;

use crewmux_mux::{ComposerState, Key};

/// Which runtime's composer we know how to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ComposerParser {
    /// Text after the last `stub> ` prompt.
    Stub,
}

impl ComposerParser {
    /// Editable composer text. `None` when this parser does not match.
    #[must_use]
    pub(crate) fn region(self, screen: &str) -> Option<String> {
        match self {
            Self::Stub => stub_region(screen),
        }
    }

    /// Prompt, draft, gate, or unknown.
    #[must_use]
    pub(crate) fn state(self, screen: &str, draft_age: Option<Duration>) -> ComposerState {
        match self {
            Self::Stub => classify_stub(screen, draft_age),
        }
    }

    /// Keys that drop a failed paste. The stub's `C-u` clears its draft.
    /// Enter is never part of the clear.
    #[must_use]
    pub(crate) fn clear_keys(self) -> &'static [Key] {
        match self {
            Self::Stub => {
                static KEYS: [Key; 1] = [Key::CtrlU];
                &KEYS
            }
        }
    }
}

fn stub_region(screen: &str) -> Option<String> {
    let idx = screen.rfind("stub> ")?;
    let start = idx.saturating_add("stub> ".len());
    Some(screen.get(start..).unwrap_or("").to_owned())
}

/// Composer text in `after`, starting at the prompt that was current in `before`.
///
/// That prompt is the last `stub> ` in the verdict-time snapshot. The same
/// occurrence (by count, not by the line above it) is used in `after`. A later
/// `stub> ` inside the payload stays in the region, and an earlier prompt is
/// not selected just because it has the same reply line above it. If that
/// occurrence has scrolled out of `after`, the last prompt in `after` is used.
#[must_use]
pub(crate) fn anchored_region(before: &str, after: &str) -> Option<String> {
    let nth = before.matches("stub> ").count().checked_sub(1)?;
    region_after_prompt(after, nth).or_else(|| stub_region(after))
}

fn region_after_prompt(screen: &str, nth: usize) -> Option<String> {
    let mut from = 0usize;
    let mut seen = 0usize;
    loop {
        let rest = screen.get(from..)?;
        let rel = rest.find("stub> ")?;
        let next = from.saturating_add(rel).saturating_add("stub> ".len());
        if seen == nth {
            return Some(screen.get(next..).unwrap_or("").to_owned());
        }
        seen = seen.saturating_add(1);
        from = next;
    }
}

/// Classify a stub pane. A gate is not a prompt. Text after `stub> ` is a draft.
#[must_use]
pub(crate) fn classify_stub(text: &str, draft_age: Option<Duration>) -> ComposerState {
    if text
        .lines()
        .any(|line| line.trim().starts_with("stub gate:"))
    {
        return ComposerState::NotAtPrompt;
    }
    let Some(idx) = text.rfind("stub> ") else {
        return ComposerState::Unknown;
    };
    let start = idx.saturating_add("stub> ".len());
    let after = text.get(start..).unwrap_or("").trim();
    if after.is_empty() {
        ComposerState::AtPrompt
    } else {
        ComposerState::Draft {
            age: draft_age.unwrap_or(Duration::ZERO),
        }
    }
}

/// True when the stub prompt has text after it.
#[must_use]
pub(crate) fn stub_drafting(text: &str) -> bool {
    matches!(classify_stub(text, None), ComposerState::Draft { .. })
}

/// The parser matched and the composer has no text.
///
/// A screen the parser does not recognise is not empty: the caller treats that
/// as a degraded guard instead of submitting.
#[must_use]
pub(crate) fn composer_empty(parser: ComposerParser, screen: &str) -> bool {
    parser
        .region(screen)
        .is_some_and(|text| text.trim().is_empty())
}
