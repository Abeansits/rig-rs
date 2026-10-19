//! One classifier per runtime. Send must not use the stub screen for Claude.

use std::time::Duration;

use crewmux_mux::ComposerState;
use crewmux_types::RuntimeKind;

use crate::claude::ClaudeScreen;
use crate::codex::CodexScreen;
use crate::types::Readiness;

/// Classify captured text for `runtime`.
///
/// Stub text is [`Readiness::Ready`] only when it contains `stub> `. A Claude
/// or Codex pane is classified by that runtime's own screen parser. This is
/// the dispatch `rig send` uses (issue #34).
#[must_use]
pub fn classify_runtime(runtime: RuntimeKind, text: &str) -> Readiness {
    match runtime {
        RuntimeKind::Stub => crate::stub::classify(text),
        RuntimeKind::ClaudeCode => crate::claude::readiness(&crate::claude::classify_text(text)),
        RuntimeKind::Codex => crate::codex::to_readiness(crate::codex::classify(text)),
        RuntimeKind::OpenCode | RuntimeKind::Terminal => Readiness::Inconclusive,
    }
}

/// Composer state for the send guard. `None` means the daemon's stub parser.
#[must_use]
pub fn runtime_composer(
    runtime: RuntimeKind,
    text: &str,
    draft_age: Option<Duration>,
) -> Option<ComposerState> {
    match runtime {
        RuntimeKind::ClaudeCode => Some(claude_composer(text, draft_age)),
        RuntimeKind::Codex => Some(codex_composer(text, draft_age)),
        RuntimeKind::Stub | RuntimeKind::OpenCode | RuntimeKind::Terminal => None,
    }
}

fn claude_composer(text: &str, draft_age: Option<Duration>) -> ComposerState {
    match crate::claude::classify_text(text) {
        ClaudeScreen::Ready(_) => ComposerState::AtPrompt,
        ClaudeScreen::Draft => ComposerState::Draft {
            age: draft_age.unwrap_or(Duration::ZERO),
        },
        ClaudeScreen::Busy
        | ClaudeScreen::Dialog(_)
        | ClaudeScreen::Login(_)
        | ClaudeScreen::UnknownMenu { .. } => ComposerState::NotAtPrompt,
        ClaudeScreen::Inconclusive => ComposerState::Unknown,
    }
}

fn codex_composer(text: &str, draft_age: Option<Duration>) -> ComposerState {
    match crate::codex::classify(text) {
        CodexScreen::Ready(_) => ComposerState::AtPrompt,
        CodexScreen::Draft { .. } => ComposerState::Draft {
            age: draft_age.unwrap_or(Duration::ZERO),
        },
        CodexScreen::Busy
        | CodexScreen::Dialog(_)
        | CodexScreen::Login(_)
        | CodexScreen::UnknownMenu { .. }
        | CodexScreen::StaleServer
        | CodexScreen::ReturnedToShell
        | CodexScreen::HardFailure { .. } => ComposerState::NotAtPrompt,
        CodexScreen::Inconclusive => ComposerState::Unknown,
    }
}
