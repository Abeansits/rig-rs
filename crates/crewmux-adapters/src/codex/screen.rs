//! Positive-match Codex screen classifier.
//!
//! Ready requires a composer and `? for shortcuts`, with no interrupt hint and
//! no option menu. Login and known dialogs are matched on their own text.

use crewmux_types::LoginKind;

use crate::codex::timeutil::semvers;
use crate::dialog::{KnownDialog, menu_options};

/// Composer evidence on a ready screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerState {
    /// The footer line that contained `? for shortcuts`.
    pub footer: String,
}

/// One classification of a Codex pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodexScreen {
    /// Composer plus the shortcuts footer.
    Ready(ComposerState),
    /// A turn is in progress (`esc to interrupt`).
    Busy,
    /// A known dialog, including ones that must not be answered.
    Dialog(KnownDialog),
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

/// Classify `text`. Login and hard failures win over a composer.
#[must_use]
pub fn classify(text: &str) -> CodexScreen {
    if let Some(screen) = hard_failure(text) {
        return screen;
    }
    if stale(text) {
        return CodexScreen::StaleServer;
    }
    if returned_to_shell(text) {
        return CodexScreen::ReturnedToShell;
    }
    if let Some(kind) = login_kind(text) {
        return CodexScreen::Login(kind);
    }
    if let Some(dialog) = known_dialog(text) {
        return CodexScreen::Dialog(dialog);
    }
    if busy(text) {
        return CodexScreen::Busy;
    }
    if !menu_options(text).is_empty() {
        return CodexScreen::UnknownMenu {
            evidence: clip(text),
        };
    }
    if let Some(footer) = ready_footer(text) {
        return CodexScreen::Ready(ComposerState { footer });
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

fn known_dialog(text: &str) -> Option<KnownDialog> {
    update_dialog(text)
        .or_else(|| background_dialog(text))
        .or_else(|| mcp_dialog(text))
}

fn update_dialog(text: &str) -> Option<KnownDialog> {
    if !text.contains("Update available") {
        return None;
    }
    let options = menu_options(text);
    let has = |label: &str| options.iter().any(|opt| opt.label == label);
    let has_update = options.iter().any(|opt| opt.label.contains("Update now"));
    if !has_update || !has("Skip") || !has("Skip until next version") {
        return None;
    }
    let versions = semvers(text);
    Some(KnownDialog::CodexUpdate {
        from: versions.first()?.clone(),
        to: versions.get(1)?.clone(),
    })
}

fn background_dialog(text: &str) -> Option<KnownDialog> {
    if !text.contains("Cannot use the background server") {
        return None;
    }
    let options = menu_options(text);
    let run = options
        .iter()
        .any(|opt| opt.label.contains("Run without daemon"));
    let cancel = options.iter().any(|opt| opt.label == "Cancel");
    if run && cancel {
        Some(KnownDialog::CodexBackgroundServer)
    } else {
        None
    }
}

fn mcp_dialog(text: &str) -> Option<KnownDialog> {
    let (server, tool) = mcp_names(text)?;
    let options = menu_options(text);
    let required = ["Allow", "Allow for this session", "Always allow", "Cancel"];
    if required
        .iter()
        .all(|label| options.iter().any(|opt| opt.label == *label))
    {
        Some(KnownDialog::CodexMcpToolApproval { server, tool })
    } else {
        None
    }
}

fn mcp_names(text: &str) -> Option<(String, String)> {
    for line in text.lines() {
        let trimmed = line
            .trim()
            .trim_start_matches(['│', '┃', '›', '❯', ' '])
            .trim();
        let Some(rest) = trimmed.strip_prefix("Allow the ") else {
            continue;
        };
        let Some((server, after)) = rest.split_once(" MCP server to run tool ") else {
            continue;
        };
        let tool = after.trim().trim_end_matches(['?', '.']);
        if !server.is_empty() && !tool.is_empty() {
            return Some((server.to_owned(), tool.to_owned()));
        }
    }
    None
}

fn busy(text: &str) -> bool {
    text.to_ascii_lowercase().contains("esc to interrupt")
}

fn ready_footer(text: &str) -> Option<String> {
    if busy(text) || !has_composer(text) {
        return None;
    }
    text.lines()
        .find(|line| line.contains("? for shortcuts"))
        .map(|line| line.trim().to_owned())
}

fn has_composer(text: &str) -> bool {
    text.contains('╭') || text.contains('│') || text.lines().any(|line| line.trim() == "›")
}

fn last_line(text: &str) -> Option<&str> {
    text.lines().rev().find(|line| !line.trim().is_empty())
}

fn clip(text: &str) -> String {
    text.chars().take(500).collect()
}
