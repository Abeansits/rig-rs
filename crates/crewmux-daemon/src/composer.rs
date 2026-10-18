//! Composer parsers. The stub parser reads `stub> `. Claude and Codex use
//! the adapter classifier for that runtime (issue #34).

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

    /// Drop whatever `C-u` left and redraw an empty prompt.
    ///
    /// `C-g` is the stub's reset. Claude Code may open `$EDITOR` for that
    /// key, so callers must use [`recovery_keys`] for a real runtime.
    #[must_use]
    pub(crate) fn reset_keys(self) -> &'static [Key] {
        match self {
            Self::Stub => {
                static KEYS: [Key; 1] = [Key::CtrlG];
                &KEYS
            }
        }
    }
}

/// Keys that recover a failed paste for `runtime`.
///
/// Only the stub gets `C-g`. Claude Code can treat that byte as "open
/// `$EDITOR`". Codex and the other runtimes have no reset key here.
#[must_use]
pub(crate) fn recovery_keys(runtime: crewmux_types::RuntimeKind) -> &'static [Key] {
    match runtime {
        crewmux_types::RuntimeKind::Stub => ComposerParser::Stub.reset_keys(),
        crewmux_types::RuntimeKind::ClaudeCode
        | crewmux_types::RuntimeKind::Codex
        | crewmux_types::RuntimeKind::OpenCode
        | crewmux_types::RuntimeKind::Terminal => &[],
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

/// Composer state for `runtime`. Claude and Codex do not go through the stub parser.
#[must_use]
pub(crate) fn classify_runtime(
    runtime: crewmux_types::RuntimeKind,
    text: &str,
    draft_age: Option<Duration>,
) -> ComposerState {
    crewmux_adapters::runtime_composer(runtime, text, draft_age)
        .unwrap_or_else(|| classify_stub(text, draft_age))
}

/// True when `runtime`'s composer holds a draft.
#[must_use]
pub(crate) fn runtime_drafting(runtime: crewmux_types::RuntimeKind, text: &str) -> bool {
    matches!(
        classify_runtime(runtime, text, None),
        ComposerState::Draft { .. }
    )
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

#[cfg(test)]
mod tests {
    use crewmux_mux::Key;
    use crewmux_types::RuntimeKind;

    use super::recovery_keys;

    #[test]
    fn failed_paste_reset_is_stub_only() {
        assert_eq!(recovery_keys(RuntimeKind::Stub), &[Key::CtrlG]);
        for runtime in [
            RuntimeKind::ClaudeCode,
            RuntimeKind::Codex,
            RuntimeKind::OpenCode,
            RuntimeKind::Terminal,
        ] {
            assert!(
                recovery_keys(runtime).is_empty(),
                "{runtime:?} must not be sent C-g"
            );
        }
    }
}
