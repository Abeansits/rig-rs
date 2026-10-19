//! Positive-match Claude screen classifier.
//!
//! Ready is the last `❯` between box-drawing rules when that prompt is empty
//! or an idle `Try "…"` hint, every line under it until the closing rule is
//! blank, and the pane is not busy. A completed `✻ <word> for <duration>` row
//! is not busy. A dialog is recognised from the view that starts at that
//! prompt, so a trust screen in the scrollback is not the current dialog.

use crewmux_mux::Capture;
use crewmux_types::{ComposerState, DialogId, ReadinessCode};

use crate::claude::composer::{
    ComposerAnchor, boxed_body, composer_anchor, composer_view, has_continuation, is_box_rule,
    is_dim_suggestion, is_idle_body,
};
use crate::claude::dialogs::CLAUDE_DIALOGS;
use crate::dialog::{MenuOption, is_option_menu, parse_menu, recognise};
use crate::sgr::strip_csi;
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
///
/// Structure is read with CSI removed. A faint suggestion is decided from the
/// raw line, which still has the SGR when the capture used escapes.
#[must_use]
pub fn classify_text(text: &str) -> ClaudeScreen {
    let plain = strip_csi(text);
    let screen = classify_plain(&plain);
    promote_faint(text, &plain, screen)
}

fn promote_faint(raw: &str, plain: &str, screen: ClaudeScreen) -> ClaudeScreen {
    if !matches!(screen, ClaudeScreen::Draft) {
        return screen;
    }
    let Some(anchor) = composer_anchor(plain) else {
        return screen;
    };
    if has_continuation(plain, &anchor) {
        return screen;
    }
    let Some(line) = raw.lines().nth(anchor.line) else {
        return screen;
    };
    if is_dim_suggestion(line) {
        ClaudeScreen::Ready(ComposerState { bordered: true })
    } else {
        screen
    }
}

fn classify_plain(text: &str) -> ClaudeScreen {
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
    if permission_dialog(view) {
        return ClaudeScreen::UnknownMenu {
            evidence: clip(text),
        };
    }
    if no_conversation(view) {
        return ClaudeScreen::UnknownMenu {
            evidence: clip(text),
        };
    }
    if let Some(kind) = login_kind(view) {
        return ClaudeScreen::Login(kind);
    }
    // A filled composer is a draft even while a spinner is still above the box.
    // The 2.1.287 x-draft pane shows `· Incubating…` over a two-row draft.
    if let Some(anchor) = anchor.as_ref()
        && !dim_suggestion(text, anchor)
        && (has_continuation(text, anchor) || !is_idle_body(&anchor.body))
    {
        return ClaudeScreen::Draft;
    }
    if has_interrupt(text) || pane_is_busy(text, anchor.as_ref()) {
        return ClaudeScreen::Busy;
    }
    if is_option_menu(&parse_menu(view)) {
        return ClaudeScreen::UnknownMenu {
            evidence: clip(text),
        };
    }
    if anchor.is_some() {
        return ClaudeScreen::Ready(ComposerState { bordered: true });
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

fn dim_suggestion(text: &str, anchor: &ComposerAnchor) -> bool {
    if has_continuation(text, anchor) {
        return false;
    }
    text.lines().nth(anchor.line).is_some_and(is_dim_suggestion)
}

/// A permission or tool-approval dialog. These are attention, never ready.
///
/// The titles are the ones Claude Code 2.1.287 printed on the live pane.
fn permission_dialog(text: &str) -> bool {
    text.contains("Read outside the working directories") || text.contains("Do you want to proceed")
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
    let Some(anchor) = composer_anchor(text) else {
        return contains_interrupt(text);
    };
    let lines: Vec<&str> = text.lines().collect();
    let start = live_band_start(&lines, anchor.line);
    lines
        .get(start..)
        .is_some_and(|band| band.iter().any(|line| contains_interrupt(line)))
}

fn contains_interrupt(text: &str) -> bool {
    text.to_ascii_lowercase().contains("esc to interrupt")
}

/// Busy only because the live region shows `esc to interrupt`, with no spinner.
#[must_use]
pub(crate) fn bounded_interrupt(text: &str) -> bool {
    has_interrupt(text) && !pane_is_busy(text, composer_anchor(text).as_ref())
}

fn live_band_start(lines: &[&str], prompt: usize) -> usize {
    let mut index = prompt;
    let mut start = prompt;
    while index > 0 {
        index = index.saturating_sub(1);
        let Some(line) = lines.get(index) else {
            break;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() || is_box_rule(line) {
            continue;
        }
        if !line.starts_with([' ', '\t']) {
            break;
        }
        start = index;
    }
    start
}

fn pane_is_busy(text: &str, anchor: Option<&ComposerAnchor>) -> bool {
    match anchor {
        Some(anchor) => live_above(text, anchor),
        None => text.lines().any(|line| is_live_line(line.trim())),
    }
}

/// Status above the composer. A newer unindented line ends an older live row.
fn live_above(text: &str, anchor: &ComposerAnchor) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    let mut index = anchor.line;
    while index > 0 {
        index = index.saturating_sub(1);
        let Some(line) = lines.get(index) else {
            return false;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() || is_box_rule(line) {
            continue;
        }
        if is_live_line(trimmed) {
            return true;
        }
        if !line.starts_with([' ', '\t']) {
            return false;
        }
    }
    false
}

fn is_live_line(trimmed: &str) -> bool {
    if trimmed.contains("· done") || completed_turn(trimmed) {
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
}

/// `✻ <words> for <duration>`. The verb may be several words. The duration is
/// any mix of hours, minutes, and seconds, including a form with no seconds.
fn completed_turn(trimmed: &str) -> bool {
    let Some(rest) = status_glyph_rest(trimmed) else {
        return false;
    };
    let rest = rest.trim_start();
    let Some(after_words) = verb_phrase_rest(rest) else {
        return false;
    };
    let Some(after_for) = after_words.strip_prefix(" for ") else {
        return false;
    };
    has_duration(after_for)
}

fn status_glyph_rest(trimmed: &str) -> Option<&str> {
    const GLYPHS: [char; 5] = ['✶', '✽', '✢', '✻', '✳'];
    let mut chars = trimmed.chars();
    let first = chars.next()?;
    GLYPHS.contains(&first).then_some(chars.as_str())
}

fn verb_phrase_rest(text: &str) -> Option<&str> {
    let mut rest = text;
    loop {
        let end = rest.find(|ch: char| !ch.is_alphabetic())?;
        if end == 0 {
            return None;
        }
        let after = rest.get(end..)?;
        let next = after.strip_prefix(' ')?;
        if next.starts_with("for ") {
            return Some(after);
        }
        if !next.starts_with(|ch: char| ch.is_alphabetic()) {
            return None;
        }
        rest = next;
    }
}

fn has_duration(text: &str) -> bool {
    let (text, hours) = take_optional(text, 'h');
    let (text, minutes) = take_optional(text, 'm');
    let (_text, seconds) = take_optional(text, 's');
    hours || minutes || seconds
}

fn take_optional(text: &str, unit: char) -> (&str, bool) {
    match take_unit(text, unit) {
        Some(rest) => (rest, true),
        None => (text, false),
    }
}

fn take_unit(text: &str, unit: char) -> Option<&str> {
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let rest = text.get(digits..)?.strip_prefix(unit)?;
    Some(rest.trim_start())
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
