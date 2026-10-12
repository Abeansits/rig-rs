//! Positive-match Codex screen classifier.
//!
//! Ready requires an empty composer and `? for shortcuts`, with no interrupt
//! hint and no option menu. A composer that already holds a draft is not
//! ready. Dialog titles match only as their own line, and an empty composer
//! wins over a dialog title left earlier in scrollback.

use crewmux_types::LoginKind;

use crate::codex::timeutil::semvers;
use crate::dialog::{KnownDialog, menu_options, trust_folder};

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
        /// Text after the cursor. A prefix of the pasted body is enough.
        visible: String,
    },
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
    if busy(text) {
        return CodexScreen::Busy;
    }
    if let Some(screen) = composer_screen(text) {
        return screen;
    }
    if let Some(dialog) = known_dialog(text) {
        return CodexScreen::Dialog(dialog);
    }
    if !menu_options(text).is_empty() {
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
    let region = last_box(text)?;
    let after = text.get(region.end..).unwrap_or("");
    if let Some(dialog) = known_dialog(after) {
        return Some(CodexScreen::Dialog(dialog));
    }
    if !menu_options(after).is_empty() {
        return Some(CodexScreen::UnknownMenu {
            evidence: clip(after),
        });
    }
    match region.draft {
        Err(()) => None,
        Ok(visible) if visible.is_empty() => footer_after(text, region.end)
            .map(|footer| CodexScreen::Ready(ComposerState { footer })),
        Ok(visible) => Some(CodexScreen::Draft { visible }),
    }
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

fn composer_draft(inside: &str) -> Result<String, ()> {
    let mut found = None;
    for line in inside.lines() {
        if let Some(draft) = cursor_draft(line) {
            found = Some(draft);
        }
    }
    found.ok_or(())
}

fn cursor_draft(line: &str) -> Option<String> {
    let rest = line.trim().trim_start_matches(['│', '┃', ' ']);
    let after = rest.strip_prefix('›').or_else(|| rest.strip_prefix('❯'))?;
    let after = after.strip_prefix(' ').unwrap_or(after);
    Some(after.trim_end_matches(['│', '┃', ' ']).to_owned())
}

fn footer_after(text: &str, end: usize) -> Option<String> {
    text.get(end..)?
        .lines()
        .find(|line| line.contains("? for shortcuts"))
        .map(|line| line.trim().to_owned())
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
        .or_else(|| trust_dialog(text))
}

fn has_title(text: &str, title: &str) -> bool {
    text.lines().any(|line| {
        let rest = line
            .trim()
            .trim_start_matches(|ch: char| !ch.is_ascii_alphanumeric());
        rest.starts_with(title)
    })
}

fn update_dialog(text: &str) -> Option<KnownDialog> {
    if !has_title(text, "Update available") {
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
    if !has_title(text, "Cannot use the background server") {
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

fn trust_dialog(text: &str) -> Option<KnownDialog> {
    if !has_title(text, "Accessing workspace") {
        return None;
    }
    let options = menu_options(text);
    let yes = options
        .iter()
        .any(|opt| opt.label.contains("Yes, I trust this folder"));
    let no = options.iter().any(|opt| opt.label.contains("No, exit"));
    if yes && no {
        Some(KnownDialog::ClaudeTrust {
            folder: trust_folder(text),
        })
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

fn last_line(text: &str) -> Option<&str> {
    text.lines().rev().find(|line| !line.trim().is_empty())
}

fn clip(text: &str) -> String {
    text.chars().take(500).collect()
}
