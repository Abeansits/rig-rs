//! Positive-match Codex screen classifier.
//!
//! Ready requires an empty composer and `? for shortcuts`. Any non-empty
//! continuation inside the composer is a draft. A finished `Worked for` line
//! is idle. A live `esc to interrupt` or `◦ Working` row is busy. Login, a
//! stale server, and a missing Codex home match the current screen.

use crewmux_types::{DialogId, LoginKind};

use crate::claude::CLAUDE_DIALOGS;
use crate::codex::dialogs::CODEX_DIALOGS;
use crate::dialog::{is_option_menu, parse_menu, recognise};

/// Composer evidence on a ready screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerState {
    /// The footer line that contained `? for shortcuts`.
    pub footer: String,
}

/// One classification of a Codex pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodexScreen {
    /// Empty composer plus the shortcuts footer.
    Ready(ComposerState),
    /// The composer holds a non-empty draft. Not ready to submit.
    Draft {
        /// Non-empty text inside the box. A prefix of the pasted body is enough.
        visible: String,
    },
    /// A turn is in progress.
    Busy,
    /// A recognised dialog.
    Dialog(DialogId),
    /// A login flow. No key is sent.
    Login(LoginKind),
    /// The app-server control socket is already in use.
    StaleServer,
    /// A numbered menu that is not a known dialog.
    UnknownMenu {
        /// Captured text, clipped.
        evidence: String,
    },
    /// The pane is back at a shell prompt.
    ReturnedToShell,
    /// `CODEX_HOME` is missing.
    HardFailure {
        /// Stable reason token.
        reason: String,
    },
    /// Neither a gate nor a composer.
    Inconclusive,
}

/// Classify `text`.
#[must_use]
pub fn classify(text: &str) -> CodexScreen {
    let live = live_screen(text);
    if let Some(screen) = hard_failure(live) {
        return screen;
    }
    if stale(live) {
        return CodexScreen::StaleServer;
    }
    if returned_to_shell(text) {
        return CodexScreen::ReturnedToShell;
    }
    if let Some(kind) = login_kind(live) {
        return CodexScreen::Login(kind);
    }
    if busy(text) {
        return CodexScreen::Busy;
    }
    if let Some(screen) = composer_screen(text) {
        return screen;
    }
    if let Some(screen) = recognised(text) {
        return screen;
    }
    if is_option_menu(&parse_menu(text)) {
        return CodexScreen::UnknownMenu {
            evidence: clip(text),
        };
    }
    CodexScreen::Inconclusive
}

/// A not-logged-in pre-flight overrides every screen, including a composer.
#[must_use]
pub fn apply_preflight(preflight: &crate::codex::Preflight, screen: CodexScreen) -> CodexScreen {
    if matches!(preflight, crate::codex::Preflight::NotLoggedIn) {
        CodexScreen::Login(LoginKind::PreflightNotLoggedIn)
    } else {
        screen
    }
}

struct BoxRegion {
    end: usize,
    draft: Result<String, ()>,
}

fn composer_screen(text: &str) -> Option<CodexScreen> {
    if let Some(region) = cursor_box(text) {
        let after = text.get(region.end..).unwrap_or("");
        if let Some(dialog) = recognised(after) {
            return Some(dialog);
        }
        if is_option_menu(&parse_menu(after)) {
            return Some(CodexScreen::UnknownMenu {
                evidence: clip(after),
            });
        }
        return draft_or_ready(region.draft, text, region.end);
    }
    let search = after_last_box(text);
    if let Some(prompt) = composer_prompt(search) {
        if let Some(dialog) = recognised(search) {
            return Some(dialog);
        }
        return Some(prompt_screen(search, prompt));
    }
    plain_box_draft(text)
}

fn draft_or_ready(draft: Result<String, ()>, text: &str, end: usize) -> Option<CodexScreen> {
    match draft {
        Err(()) => None,
        Ok(visible) if prompt_is_empty(&visible) => {
            footer_after(text, end).map(|footer| CodexScreen::Ready(ComposerState { footer }))
        }
        Ok(visible) => Some(CodexScreen::Draft { visible }),
    }
}

fn prompt_screen(search: &str, prompt: String) -> CodexScreen {
    let footer = footer_after(search, 0).unwrap_or_default();
    if prompt_is_empty(&prompt) && !has_continuation(search) {
        CodexScreen::Ready(ComposerState { footer })
    } else {
        CodexScreen::Draft {
            visible: if prompt_is_empty(&prompt) {
                continuation_text(search)
            } else {
                prompt
            },
        }
    }
}

fn cursor_box(text: &str) -> Option<BoxRegion> {
    let region = last_box(text)?;
    let inside = text.get(..region.end).unwrap_or("");
    let start = inside.rfind('╭')?;
    let body = inside.get(start..).unwrap_or("");
    if !body.contains('›') && !body.contains('❯') {
        return None;
    }
    Some(region)
}

fn last_box(text: &str) -> Option<BoxRegion> {
    let start = text.rfind('╭')?;
    let tail = text.get(start..)?;
    let Some(rel_close) = tail.find('╰') else {
        return Some(BoxRegion {
            end: text.len(),
            draft: Err(()),
        });
    };
    let close = start.saturating_add(rel_close);
    let inside = text.get(start..close).unwrap_or("");
    let end = text
        .get(close..)
        .and_then(|rest| rest.find('\n'))
        .map_or(text.len(), |index| {
            close.saturating_add(index).saturating_add(1)
        });
    Some(BoxRegion {
        end,
        draft: composer_draft(inside),
    })
}

fn after_last_box(text: &str) -> &str {
    let Some(start) = text.rfind('╭') else {
        return text;
    };
    let tail = text.get(start..).unwrap_or(text);
    let Some(rel) = tail.find('╰') else {
        return text;
    };
    let close = start.saturating_add(rel);
    text.get(close..)
        .and_then(|rest| rest.find('\n').map(|index| index.saturating_add(1)))
        .and_then(|index| text.get(close.saturating_add(index)..))
        .unwrap_or("")
}

fn plain_box_draft(text: &str) -> Option<CodexScreen> {
    let region = last_box(text)?;
    match region.draft {
        Ok(visible) if !visible.is_empty() => Some(CodexScreen::Draft { visible }),
        Ok(_) | Err(()) => None,
    }
}

fn recognised(text: &str) -> Option<CodexScreen> {
    let found = recognise(CODEX_DIALOGS, text).or_else(|| recognise(CLAUDE_DIALOGS, text))?;
    Some(CodexScreen::Dialog(found.spec.id))
}

fn composer_draft(inside: &str) -> Result<String, ()> {
    let mut parts = Vec::new();
    let mut saw_cursor = false;
    for line in inside.lines() {
        if let Some(draft) = cursor_draft(line) {
            saw_cursor = true;
            if !prompt_is_empty(&draft) {
                parts.push(draft);
            }
            continue;
        }
        if let Some(text) = plain_box_line(line) {
            parts.push(text);
        }
    }
    if parts.is_empty() {
        if saw_cursor {
            Ok(String::new())
        } else {
            Err(())
        }
    } else {
        Ok(parts.join("\n"))
    }
}

fn plain_box_line(line: &str) -> Option<String> {
    let text = line
        .trim()
        .trim_matches(['│', '┃', '╭', '╮', '╰', '╯', '─', ' ']);
    if text.is_empty() {
        None
    } else {
        Some(text.to_owned())
    }
}

fn cursor_draft(line: &str) -> Option<String> {
    let rest = line.trim().trim_start_matches(['│', '┃', ' ']);
    let after = rest.strip_prefix('›').or_else(|| rest.strip_prefix('❯'))?;
    if after
        .trim_start()
        .starts_with(|ch: char| ch.is_ascii_digit())
        && after.contains('.')
    {
        return None;
    }
    let after = after.strip_prefix(' ').unwrap_or(after);
    Some(after.trim_end_matches(['│', '┃', ' ']).to_owned())
}

fn composer_prompt(text: &str) -> Option<String> {
    let mut found = None;
    for line in text.lines() {
        if let Some(draft) = cursor_draft(line) {
            found = Some(draft);
        }
    }
    found
}

fn prompt_is_empty(visible: &str) -> bool {
    let trimmed = visible.trim();
    trimmed.is_empty() || trimmed == "Ask Codex to do anything"
}

fn has_continuation(text: &str) -> bool {
    !continuation_text(text).is_empty()
}

fn continuation_text(text: &str) -> String {
    let mut after_prompt = false;
    let mut parts = Vec::new();
    for line in text.lines() {
        if cursor_draft(line).is_some() {
            after_prompt = true;
            parts.clear();
            continue;
        }
        if !after_prompt || is_status_line(line) {
            continue;
        }
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_owned());
        }
    }
    parts.join("\n")
}

fn is_status_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.contains("? for shortcuts")
        || trimmed.starts_with("GPT-")
        || trimmed.starts_with("OpenAI Codex")
        || trimmed.starts_with(">_")
        || trimmed.starts_with('•')
        || trimmed.starts_with('◦')
}

fn footer_after(text: &str, end: usize) -> Option<String> {
    text.get(end..)?
        .lines()
        .find(|line| line.contains("? for shortcuts"))
        .map(|line| line.trim().to_owned())
}

fn live_screen(text: &str) -> &str {
    text.rfind('╭')
        .and_then(|start| text.get(start..))
        .unwrap_or(text)
}

fn hard_failure(text: &str) -> Option<CodexScreen> {
    let lower = text.to_ascii_lowercase();
    let missing = lower.contains("codex home")
        && (lower.contains("does not exist") || lower.contains("error finding"));
    if missing {
        Some(CodexScreen::HardFailure {
            reason: "codex_home_missing".to_owned(),
        })
    } else {
        None
    }
}

fn stale(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("control socket is already in use")
        || (lower.contains("app-server") && lower.contains("crash"))
}

fn returned_to_shell(text: &str) -> bool {
    if text.contains("? for shortcuts") {
        return false;
    }
    last_line(text).is_some_and(is_shell_prompt)
}

fn is_shell_prompt(line: &str) -> bool {
    let line = line.trim();
    line == "$" || line.ends_with(" $")
}

fn login_kind(text: &str) -> Option<LoginKind> {
    if text.contains("Finish signing in via your browser") {
        return Some(LoginKind::ChatGptBrowser);
    }
    if text.contains("Enter this one-time code") {
        return Some(LoginKind::DeviceCode);
    }
    if text.contains("Paste or type your API key") {
        return Some(LoginKind::ApiKeyEntry);
    }
    if text.contains("Sign in with ChatGPT") && text.contains("Provide your own API key") {
        return Some(LoginKind::Menu);
    }
    None
}

fn busy(text: &str) -> bool {
    text.lines().any(|line| {
        let lower = line.to_ascii_lowercase();
        if lower.contains("worked for") {
            return false;
        }
        lower.contains("esc to interrupt") || line.contains('◦')
    })
}

fn last_line(text: &str) -> Option<&str> {
    text.lines().rev().find(|line| !line.trim().is_empty())
}

fn clip(text: &str) -> String {
    text.chars().take(500).collect()
}
