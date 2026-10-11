//! Runtime kind, launch posture, and fork source.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Which agent runtime a seat launches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RuntimeKind {
    /// Claude Code.
    #[serde(rename = "claude-code")]
    ClaudeCode,
    /// Codex.
    #[serde(rename = "codex")]
    Codex,
    /// `OpenCode`.
    #[serde(rename = "opencode")]
    OpenCode,
    /// A plain terminal.
    #[serde(rename = "terminal")]
    Terminal,
    /// The in-tree stub fixture.
    #[serde(rename = "stub")]
    Stub,
}

impl RuntimeKind {
    /// Stable wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::Terminal => "terminal",
            Self::Stub => "stub",
        }
    }
}

/// How much permission a launch asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchPosture {
    /// The floor posture.
    Floor,
    /// Full bypass of first-launch warnings.
    FullBypass,
    /// Adapter default.
    Auto,
}

/// Where a forked session's identity comes from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForkSource {
    /// A provider-native session id.
    NativeId(String),
    /// A path to a stored artifact.
    ArtifactPath(PathBuf),
    /// A seat or session name.
    Name(String),
    /// The most recent session.
    Last,
}
