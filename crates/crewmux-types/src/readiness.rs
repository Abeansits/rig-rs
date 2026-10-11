//! Readiness codes shared by adapters and seat state.

use serde::{Deserialize, Serialize};

/// Why a seat is or is not ready for input.
///
/// `Ready` requires a positive composer match. Gate codes, including the ones
/// added for first-run menus, are attention.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessCode {
    /// The composer matched.
    Ready,
    /// The runtime is still starting.
    Booting,
    /// A trust prompt is on screen.
    TrustGate,
    /// A hook-trust prompt is on screen.
    HookTrustGate,
    /// An update prompt is on screen.
    UpdateGate,
    /// The runtime wants a login.
    LoginRequired,
    /// An MCP approval prompt is on screen.
    McpGate,
    /// A bypass-consent prompt is on screen.
    BypassConsentGate,
    /// Codex refused to refresh a stored token.
    CodexAuthRefusal,
    /// The Codex client is incompatible.
    CodexClientIncompatible,
    /// Claude's first-run theme picker.
    OnboardingTheme,
    /// A numbered menu that is not a known gate.
    UnknownMenu,
    /// A stable screen that matched nothing.
    UnrecognisedScreen,
    /// A caller-supplied code, such as an invalid working directory.
    Other(String),
}

impl ReadinessCode {
    /// Wire token. `Other` uses its string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Ready => "ready",
            Self::Booting => "booting",
            Self::TrustGate => "trust_gate",
            Self::HookTrustGate => "hook_trust_gate",
            Self::UpdateGate => "update_gate",
            Self::LoginRequired => "login_required",
            Self::McpGate => "mcp_gate",
            Self::BypassConsentGate => "bypass_consent_gate",
            Self::CodexAuthRefusal => "codex_auth_refusal",
            Self::CodexClientIncompatible => "codex_client_incompatible",
            Self::OnboardingTheme => "onboarding_theme",
            Self::UnknownMenu => "unknown_menu",
            Self::UnrecognisedScreen => "unrecognised_screen",
            Self::Other(text) => text.as_str(),
        }
    }

    /// True for every code except [`Ready`](Self::Ready) and [`Booting`](Self::Booting).
    ///
    /// This is the attention set: the historical gate codes plus
    /// `OnboardingTheme`, `UnknownMenu`, and `UnrecognisedScreen`.
    #[must_use]
    pub const fn requires_attention(&self) -> bool {
        !matches!(self, Self::Ready | Self::Booting)
    }

    /// Parse a wire token. Unknown tokens become [`Other`](Self::Other).
    #[must_use]
    pub fn parse(text: &str) -> Self {
        match text {
            "ready" => Self::Ready,
            "booting" => Self::Booting,
            "trust_gate" => Self::TrustGate,
            "hook_trust_gate" => Self::HookTrustGate,
            "update_gate" => Self::UpdateGate,
            "login_required" => Self::LoginRequired,
            "mcp_gate" => Self::McpGate,
            "bypass_consent_gate" => Self::BypassConsentGate,
            "codex_auth_refusal" => Self::CodexAuthRefusal,
            "codex_client_incompatible" => Self::CodexClientIncompatible,
            "onboarding_theme" => Self::OnboardingTheme,
            "unknown_menu" => Self::UnknownMenu,
            "unrecognised_screen" => Self::UnrecognisedScreen,
            other => Self::Other(other.to_owned()),
        }
    }
}
