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
    /// `esc to interrupt` is on screen, or a spinner with no interrupt hint.
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
            Self::Dialog(dialog) => dialog_code(&dialog.known),
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
    let options = menu_lines(text);
    if let Some(dialog) = identify_dialog(text, &options)
        && is_claude(&dialog.known)
    {
        return ClaudeScreen::Dialog(dialog);
    }
    if no_conversation(text) {
        return ClaudeScreen::UnknownMenu {
            evidence: clip(text),
        };
    }
    if let Some(kind) = login_kind(text) {
        return ClaudeScreen::Login(kind);
    }
    if has_interrupt(text) || spinning(text) {
        return ClaudeScreen::Busy;
    }
    if is_option_menu(&options) {
        return ClaudeScreen::UnknownMenu {
            evidence: clip(text),
        };
    }
    if crate::claude::composer::is_draft(text) {
        return ClaudeScreen::Draft;
    }
    if crate::claude::composer::is_empty_composer(text) {
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
        | ClaudeScreen::Draft
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
    if let Some(name) = mcp_name(text, options) {
        return Some(KnownDialog::ClaudeNewMcpServer { name });
    }
    if line_has_title(text, "Try the new fullscreen renderer?")
        && has_option(options, "1. Yes, try it")
        && has_option(options, "2. Not now")
    {
        return Some(KnownDialog::ClaudeFullscreenRenderer);
    }
    if line_has_title(text, "Bypass Permissions")
        && has_option(options, "No, exit")
        && has_option(options, "Yes, I accept")
    {
        return Some(KnownDialog::ClaudeBypassConsent);
    }
    if trust_title(text) && has_option(options, "No, exit") && has_option(options, TRUST_YES) {
        return Some(KnownDialog::ClaudeTrust);
    }
    if line_has_title(text, "Update available")
        && options.iter().any(|line| line.text.contains("Update now"))
        && options.iter().any(|line| line.text.contains("Skip"))
    {
        return Some(KnownDialog::CodexUpdate);
    }
    None
}

const TRUST_YES: &str = "Yes, I trust this folder";

fn trust_title(text: &str) -> bool {
    line_has_title(text, "Do you trust the files in this folder")
        || line_has_title(text, "Quick safety check:")
        || line_has_title(text, "Is this a project you created or one you trust?")
        || line_has_title(text, "Accessing workspace")
}

/// Folder path on a trust screen. `None` when the title has no path.
#[must_use]
pub(crate) fn trust_folder(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("Accessing workspace") else {
            continue;
        };
        let rest = rest.trim().trim_start_matches(':').trim();
        if rest.starts_with('/') {
            return Some(rest.to_owned());
        }
        let later = lines.get(index.saturating_add(1)..).unwrap_or(&[]);
        for next in later {
            let next = next.trim();
            if next.is_empty() {
                continue;
            }
            if next.starts_with('/') {
                return Some(next.to_owned());
            }
            break;
        }
    }
    None
}

/// The screen path and the seat cwd name the same directory.
#[must_use]
pub(crate) fn same_dir(screen_path: &str, cwd: &std::path::Path) -> bool {
    trim_dir(screen_path) == trim_dir(&cwd.display().to_string())
}

fn trim_dir(path: &str) -> String {
    let trimmed = path.trim().trim_end_matches('/');
    if trimmed.is_empty() { "/" } else { trimmed }.to_owned()
}

fn mcp_name(text: &str, options: &[MenuLine]) -> Option<String> {
    for line in text.lines() {
        let Some(rest) = line
            .trim()
            .strip_prefix("New MCP server found in this project:")
        else {
            continue;
        };
        let name = rest.trim().trim_end_matches('.');
        if !name.is_empty() && has_option(options, "Continue without using this MCP server") {
            return Some(name.to_owned());
        }
    }
    if !mcp_banner(text) || options.is_empty() {
        return None;
    }
    let name = options
        .first()
        .map(|line| {
            line.text
                .trim()
                .trim_start_matches("[✔]")
                .trim()
                .trim_start_matches("[ ]")
                .trim()
                .to_owned()
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "project".to_owned());
    Some(name)
}

fn mcp_banner(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim();
        if trimmed.starts_with("New MCP server found") {
            return true;
        }
        let digit = trimmed.chars().next().is_some_and(|ch| ch.is_ascii_digit());
        trimmed.contains("MCP servers found in .mcp.json") && (digit || trimmed.starts_with("MCP"))
    })
}

fn has_option(options: &[MenuLine], text: &str) -> bool {
    options
        .iter()
        .any(|line| line.text == text || option_label(&line.text) == text)
}

fn option_label(text: &str) -> &str {
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return text;
    }
    text.get(digits.len()..)
        .and_then(|rest| rest.strip_prefix(". "))
        .unwrap_or(text)
}

/// A title is the start of a line, after an optional `WARNING:` prefix.
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

fn is_claude(kind: &KnownDialog) -> bool {
    !matches!(kind, KnownDialog::CodexUpdate)
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

fn has_interrupt(text: &str) -> bool {
    text.to_ascii_lowercase().contains("esc to interrupt")
}

fn spinning(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim();
        if trimmed.contains("Crunched") {
            return false;
        }
        trimmed.chars().any(is_spinner)
            || (trimmed.contains('…')
                && (trimmed.starts_with("Working")
                    || trimmed.contains("Thinking")
                    || trimmed.contains("Onioning")
                    || trimmed.contains("Sketching")
                    || trimmed.contains("Processing")))
    })
}

fn is_spinner(ch: char) -> bool {
    ('\u{2800}'..='\u{28ff}').contains(&ch) || matches!(ch, '✶' | '✽')
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
