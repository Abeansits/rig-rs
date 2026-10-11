//! Seat lifecycle and readiness. Statuses are enums, never raw strings on the wire.

use serde::{Deserialize, Serialize};

/// Why a seat needs a person. Includes the PE-001 and PE-002 codes slice 1 will classify.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessCode {
    /// Positive composer match. Not an attention reason.
    Ready,
    /// Still starting.
    Booting,
    /// Claude trust prompt.
    TrustGate,
    /// Hook trust prompt.
    HookTrustGate,
    /// Update prompt.
    UpdateGate,
    /// Login or sign-in prompt.
    LoginRequired,
    /// MCP approval prompt.
    McpGate,
    /// Bypass-permissions consent.
    BypassConsentGate,
    /// Codex could not refresh its token.
    CodexAuthRefusal,
    /// Codex client is too old.
    CodexClientIncompatible,
    /// First-run theme picker (PE-001).
    OnboardingTheme,
    /// Numbered menu that matches no known gate (PE-001).
    UnknownMenu,
    /// Stable screen that never became ready (PE-002).
    UnrecognisedScreen,
    /// A code this build does not name.
    Other(String),
}

impl ReadinessCode {
    /// Whether startup must stop and show the seat in the attention section.
    #[must_use]
    pub fn requires_attention(&self) -> bool {
        !matches!(self, Self::Ready | Self::Booting)
    }
}

/// Seat lifecycle. `Attention` carries the readiness code so the TUI can show it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeatState {
    /// Spec accepted, pane not started.
    Planned,
    /// Pane is starting.
    Launching,
    /// Composer is ready.
    Ready,
    /// A person has to act. The code is the reason.
    Attention(ReadinessCode),
    /// Stopped on purpose.
    Stopped,
    /// The pane died.
    Dead,
}

impl SeatState {
    /// Short label for the explorer row.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Launching => "launching",
            Self::Ready => "ready",
            Self::Attention(_) => "attention",
            Self::Stopped => "stopped",
            Self::Dead => "dead",
        }
    }

    /// Attention reason, when this state is [`SeatState::Attention`].
    #[must_use]
    pub fn attention_code(&self) -> Option<&ReadinessCode> {
        match self {
            Self::Attention(code) => Some(code),
            Self::Planned | Self::Launching | Self::Ready | Self::Stopped | Self::Dead => None,
        }
    }

    /// A selected seat can be attached when it still has a live session.
    #[must_use]
    pub fn can_attach(&self) -> bool {
        matches!(self, Self::Ready | Self::Attention(_) | Self::Launching)
    }
}

/// Which external launcher opens a seat when tmux would otherwise nest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LauncherKind {
    /// Herdr control socket. Attach-only: never start or stop the Herdr server.
    Herdr,
    /// Ghostty window via `osascript` on macOS.
    Ghostty,
}

impl LauncherKind {
    /// Parse `herdr` or `ghostty`.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "herdr" => Some(Self::Herdr),
            "ghostty" => Some(Self::Ghostty),
            _ => None,
        }
    }
}

/// Runtime that owns the seat. Stub is the v1 fixture runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    /// Claude Code.
    ClaudeCode,
    /// Codex.
    Codex,
    /// Open-code agent runtime.
    OpenCode,
    /// Plain terminal.
    Terminal,
    /// Deterministic test runtime.
    Stub,
}

impl RuntimeKind {
    /// Explorer label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::Terminal => "terminal",
            Self::Stub => "stub",
        }
    }
}
