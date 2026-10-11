//! Stub composer. Claude and Codex parsers are later slices.
//!
//! `Unknown` is a degraded guard: only copy mode and the typing rule run.

use std::time::Duration;

use crewmux_mux::ComposerState;

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
