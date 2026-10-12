//! Positive-match Claude screen classifier.
//!
//! This is not a port of `assessNativeResumeProbe`. Ready requires the composer
//! border and a `? for shortcuts` footer, with no interrupt hint and no option menu.

use crewmux_mux::Capture;
use crewmux_types::{ComposerState, KnownDialog, ReadinessCode};

use crate::screen_common::{MenuLine, is_option_menu, menu_lines};
use crate::types::Readiness;

/// A classified known dialog, including the choice list and the cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassifiedDialog {
    /// Which dialog matched.
    pub known: KnownDialog,
    /// Choices in screen order.
    pub options: Vec<MenuLine>,
    /// Index of the cursor choice.
    pub cursor: Option<usize>,
}

/// Why a Claude login screen is blocking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginKind {
    /// First-run theme picker.
    ThemePicker,
    /// "Select login method:".
    LoginMethod,
    /// OAuth code paste.
    OAuthPaste,
}

/// One classification of a Claude pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClaudeScreen {
    /// Composer and shortcuts footer, and nothing that would eat a key.
    Ready(ComposerState),
    /// `esc to interrupt` is on screen.
    Busy,
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
            Self::Dialog(_) | Self::Login(_) | Self::UnknownMenu { .. }
        )
    }

    /// Readiness code for a gate. Ready, busy, and inconclusive have none.
    #[must_use]
    pub fn code(&self) -> Option<ReadinessCode> {
        Some(match self {
            Self::Dialog(dialog) => dialog_code(&dialog.known),
            Self::Login(LoginKind::ThemePicker) => ReadinessCode::OnboardingTheme,
            Self::Login(LoginKind::LoginMethod | LoginKind::OAuthPaste) => {
                ReadinessCode::LoginRequired
            }
            Self::UnknownMenu { .. } => ReadinessCode::UnknownMenu,
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
    let options = menu_lines(text);
    if let Some(dialog) = identify_dialog(text, &options)
        && is_claude(&dialog.known)
    {
        return ClaudeScreen::Dialog(dialog);
    }
    if let Some(kind) = login_kind(text) {
        return ClaudeScreen::Login(kind);
    }
    if has_interrupt(text) {
        return ClaudeScreen::Busy;
    }
    if is_option_menu(&options) {
        return ClaudeScreen::UnknownMenu {
            evidence: clip(text),
        };
    }
    if is_composer(text) {
        return ClaudeScreen::Ready(ComposerState { bordered: true });
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
        ClaudeScreen::Dialog(dialog)
            if matches!(dialog.known, KnownDialog::ClaudeNewMcpServer { .. }) =>
        {
            Some("mcp dialog appeared despite --strict-mcp-config")
        }
        ClaudeScreen::Ready(_)
        | ClaudeScreen::Busy
        | ClaudeScreen::Dialog(_)
        | ClaudeScreen::Login(_)
        | ClaudeScreen::UnknownMenu { .. }
        | ClaudeScreen::Inconclusive => None,
    }
}

/// Title plus the full option list. `None` when either side is missing.
#[must_use]
pub fn identify_dialog(text: &str, options: &[MenuLine]) -> Option<ClassifiedDialog> {
    let known = dialog_kind(text, options)?;
    Some(ClassifiedDialog {
        known,
        options: options.to_vec(),
        cursor: crate::screen_common::cursor_index(options),
    })
}

fn dialog_kind(text: &str, options: &[MenuLine]) -> Option<KnownDialog> {
    if let Some(name) = mcp_name(text)
        && has_option(options, "Continue without using this MCP server")
    {
        return Some(KnownDialog::ClaudeNewMcpServer { name });
    }
    if text.contains("Try the new fullscreen renderer?")
        && has_option(options, "1. Yes, try it")
        && has_option(options, "2. Not now")
    {
        return Some(KnownDialog::ClaudeFullscreenRenderer);
    }
    if text.contains("Bypass Permissions")
        && has_option(options, "No, exit")
        && has_option(options, "Yes, I accept")
    {
        return Some(KnownDialog::ClaudeBypassConsent);
    }
    if trust_title(text) && has_option(options, "No, exit") && has_option(options, TRUST_YES) {
        return Some(KnownDialog::ClaudeTrust);
    }
    if text.contains("Update available")
        && options.iter().any(|line| line.text.contains("Update now"))
        && options.iter().any(|line| line.text.contains("Skip"))
    {
        return Some(KnownDialog::CodexUpdate);
    }
    None
}

const TRUST_YES: &str = "Yes, I trust this folder";

fn trust_title(text: &str) -> bool {
    text.contains("Do you trust the files in this folder") || text.contains("Accessing workspace")
}

fn mcp_name(text: &str) -> Option<String> {
    for line in text.lines() {
        let Some(rest) = line
            .trim()
            .strip_prefix("New MCP server found in this project:")
        else {
            continue;
        };
        let name = rest.trim().trim_end_matches('.');
        if !name.is_empty() {
            return Some(name.to_owned());
        }
    }
    None
}

fn has_option(options: &[MenuLine], text: &str) -> bool {
    options.iter().any(|line| line.text == text)
}

fn is_claude(kind: &KnownDialog) -> bool {
    !matches!(kind, KnownDialog::CodexUpdate)
}

fn login_kind(text: &str) -> Option<LoginKind> {
    if text.contains("Choose the text style that looks best") {
        return Some(LoginKind::ThemePicker);
    }
    if text.contains("Select login method:") {
        return Some(LoginKind::LoginMethod);
    }
    if text.contains("Paste the authorization code") {
        return Some(LoginKind::OAuthPaste);
    }
    None
}

fn has_interrupt(text: &str) -> bool {
    text.to_ascii_lowercase().contains("esc to interrupt")
}

fn is_composer(text: &str) -> bool {
    text.contains("? for shortcuts") && has_border(text) && has_prompt(text)
}

fn has_border(text: &str) -> bool {
    text.lines().any(|line| {
        line.contains('╭')
            || line.contains('╰')
            || line.chars().filter(|ch| *ch == '─').take(4).count() >= 4
    })
}

fn has_prompt(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim();
        trimmed == ">" || trimmed == "❯" || (line.contains('│') && line.contains('>'))
    })
}

fn dialog_code(kind: &KnownDialog) -> ReadinessCode {
    match kind {
        KnownDialog::ClaudeTrust => ReadinessCode::TrustGate,
        KnownDialog::ClaudeBypassConsent => ReadinessCode::BypassConsentGate,
        KnownDialog::ClaudeNewMcpServer { .. } => ReadinessCode::McpGate,
        KnownDialog::ClaudeFullscreenRenderer => ReadinessCode::FullscreenRenderer,
        KnownDialog::CodexUpdate => ReadinessCode::UpdateGate,
    }
}

fn clip(text: &str) -> String {
    text.chars().take(2_000).collect()
}
