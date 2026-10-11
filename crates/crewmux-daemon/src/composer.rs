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
