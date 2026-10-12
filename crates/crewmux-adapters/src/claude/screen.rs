//! Positive-match Claude screen classifier.
//!
//! Ready is the last `❯` between box-drawing rules when that line is empty or
//! an idle `Try "…"` hint, and the pane is not busy. A dialog is recognised
//! from the view that starts at that prompt, so a trust screen in the
//! scrollback is not the current dialog.

use crewmux_mux::Capture;
use crewmux_types::{ComposerState, DialogId, ReadinessCode};

use crate::claude::composer::{boxed_body, composer_anchor, composer_view, is_idle_body};
use crate::claude::dialogs::CLAUDE_DIALOGS;
use crate::dialog::{MenuOption, is_option_menu, parse_menu, recognise};
use crate::types::Readiness;

/// A classified known dialog, including the choice list and the cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassifiedDialog {
    /// Which dialog matched.
    pub id: DialogId,
    /// Choices in screen order.
    pub options: Vec<MenuOption>,
    /// Index of the cursor choice.
    pub cursor: Option<usize>,
    /// MCP server name, when the title printed one.
    pub server: Option<String>,
}

/// Why a Claude login screen is blocking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginKind {
    /// First-run theme picker.
    ThemePicker,
    /// "Select login method:" or the in-session login footer.
    LoginMethod,
    /// OAuth code paste.
    OAuthPaste,
}

/// One classification of a Claude pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClaudeScreen {
    /// Empty composer, or an idle ghost hint, and nothing that would eat a key.
    Ready(ComposerState),
    /// `esc to interrupt`, or a leading spinner.
    Busy,
    /// The composer already holds text. Do not paste and do not send Enter.
    Draft,
    /// Title and the full option list matched.
    Dialog(ClassifiedDialog),
    /// Theme picker, login method, or OAuth paste.
    Login(LoginKind),
    /// A cursor choice or numbered list that is not a known dialog.
    UnknownMenu {
        /// Captured text, clipped.
        evidence: String,
    },
    /// Splash, blank, or anything else.
    Inconclusive,
}

impl ClaudeScreen {
    /// The composer matched.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        matches!(self, Self::Ready(_))
    }

    /// Gates and menus. Busy and inconclusive are not attention by themselves.
    #[must_use]
    pub const fn requires_attention(&self) -> bool {
        matches!(
            self,
            Self::Dialog(_) | Self::Login(_) | Self::UnknownMenu { .. } | Self::Draft
        )
    }

    /// Readiness code for a gate. Ready, busy, and inconclusive have none.
    #[must_use]
    pub fn code(&self) -> Option<ReadinessCode> {
        Some(match self {
            Self::Dialog(dialog) => dialog_code(dialog.id),
            Self::Login(LoginKind::ThemePicker) => ReadinessCode::OnboardingTheme,
            Self::Login(LoginKind::LoginMethod | LoginKind::OAuthPaste) => {
                ReadinessCode::LoginRequired
            }
            Self::UnknownMenu { .. } => ReadinessCode::UnknownMenu,
            Self::Draft => ReadinessCode::PromptDraft,
            Self::Ready(_) | Self::Busy | Self::Inconclusive => return None,
        })
    }
}

/// Classify `capture`. Only positive matches become ready or a known dialog.
#[must_use]
pub fn classify(capture: &Capture) -> ClaudeScreen {
    classify_text(&capture.text)
}

/// Classify captured text.
#[must_use]
pub fn classify_text(text: &str) -> ClaudeScreen {
    let anchor = composer_anchor(text);
    let view = anchor
        .as_ref()
        .map_or(text, |anchor| composer_view(text, anchor));
    if let Some(found) = recognise(CLAUDE_DIALOGS, view) {
        return ClaudeScreen::Dialog(ClassifiedDialog {
            id: found.spec.id,
            server: found.params.server,
            cursor: found.menu.iter().position(|option| option.selected),
            options: found.menu,
        });
    }
    if no_conversation(view) {
        return ClaudeScreen::UnknownMenu {
            evidence: clip(text),
        };
    }
    if let Some(kind) = login_kind(view) {
        return ClaudeScreen::Login(kind);
    }
    if has_interrupt(text) || spinning(text) {
        return ClaudeScreen::Busy;
    }
    if is_option_menu(&parse_menu(view)) {
        return ClaudeScreen::UnknownMenu {
            evidence: clip(text),
        };
    }
    if let Some(anchor) = anchor {
        if is_idle_body(&anchor.body) {
            return ClaudeScreen::Ready(ComposerState { bordered: true });
        }
        return ClaudeScreen::Draft;
    }
    if let Some(body) = boxed_body(text) {
        if squash_empty(&body) && text.contains("? for shortcuts") && has_box_border(text) {
            return ClaudeScreen::Ready(ComposerState { bordered: true });
        }
        if !squash_empty(&body) {
            return ClaudeScreen::Draft;
        }
    }
    ClaudeScreen::Inconclusive
}

/// Map a Claude screen onto the shared readiness enum.
#[must_use]
pub fn readiness(screen: &ClaudeScreen) -> Readiness {
    match screen.code() {
        Some(code) => Readiness::Gate(code),
        None if screen.is_ready() => Readiness::Ready,
        None if matches!(screen, ClaudeScreen::Busy) => Readiness::Booting,
        None => Readiness::Inconclusive,
    }
}

/// MCP approval under `--strict-mcp-config` is drift, not something we answer.
#[must_use]
pub fn strict_mcp_drift(screen: &ClaudeScreen) -> Option<&'static str> {
    match screen {
        ClaudeScreen::Dialog(dialog) if dialog.id == DialogId::ClaudeNewMcpServer => {
            Some("mcp dialog appeared despite --strict-mcp-config")
        }
        ClaudeScreen::Ready(_)
        | ClaudeScreen::Busy
        | ClaudeScreen::Draft
        | ClaudeScreen::Dialog(_)
        | ClaudeScreen::Login(_)
        | ClaudeScreen::UnknownMenu { .. }
        | ClaudeScreen::Inconclusive => None,
    }
}

fn squash_empty(body: &str) -> bool {
    body.split_whitespace().next().is_none()
}

fn has_box_border(text: &str) -> bool {
    text.lines()
        .any(|line| line.contains('╭') || line.contains('╰'))
}

fn login_kind(text: &str) -> Option<LoginKind> {
    if line_has_title(text, "Choose the text style that looks best") {
        return Some(LoginKind::ThemePicker);
    }
    if line_has_title(text, "Select login method:") {
        return Some(LoginKind::LoginMethod);
    }
    if line_has_title(text, "Paste the authorization code") {
        return Some(LoginKind::OAuthPaste);
    }
    if text.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with("Not logged in") && trimmed.contains("/login")
    }) {
        return Some(LoginKind::LoginMethod);
    }
    None
}

fn no_conversation(text: &str) -> bool {
    line_has_title(text, "No conversation found with session ID")
}

fn line_has_title(text: &str, title: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim();
        let trimmed = trimmed.strip_prefix("WARNING:").map_or(trimmed, str::trim);
        let Some(rest) = trimmed.strip_prefix(title) else {
            return false;
        };
        rest.is_empty() || rest.starts_with([' ', '?', ':', '.'])
    })
}

fn has_interrupt(text: &str) -> bool {
    text.to_ascii_lowercase().contains("esc to interrupt")
}

fn spinning(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim();
        if trimmed.contains("Crunched") || trimmed.contains("· done") {
            return false;
        }
        if trimmed
            .chars()
            .any(|ch| ('\u{2800}'..='\u{28ff}').contains(&ch))
        {
            return true;
        }
        if starts_with_spinner(trimmed) {
            return true;
        }
        trimmed.contains('…')
            && (trimmed.starts_with("Working")
                || trimmed.contains("Thinking")
                || trimmed.contains("Onioning")
                || trimmed.contains("Sketching")
                || trimmed.contains("Processing"))
    })
}

fn starts_with_spinner(trimmed: &str) -> bool {
    const GLYPHS: &[char] = &['✶', '✽', '✢', '✻', '✳'];
    if GLYPHS.iter().any(|glyph| trimmed.starts_with(*glyph)) {
        return true;
    }
    trimmed.starts_with("· ") && trimmed.contains('…')
}

fn dialog_code(id: DialogId) -> ReadinessCode {
    match id {
        DialogId::ClaudeTrust => ReadinessCode::TrustGate,
        DialogId::ClaudeBypassConsent => ReadinessCode::BypassConsentGate,
        DialogId::ClaudeNewMcpServer => ReadinessCode::McpGate,
        DialogId::ClaudeFullscreenRenderer => ReadinessCode::FullscreenRenderer,
        DialogId::CodexUpdate
        | DialogId::CodexMcpToolApproval
        | DialogId::CodexBackgroundServer => ReadinessCode::UpdateGate,
    }
}

fn clip(text: &str) -> String {
    text.chars().take(2_000).collect()
}
