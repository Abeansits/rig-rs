//! Labels on slice 1's [`SeatState`] and [`RuntimeKind`].
//!
//! Those types stay slice 1's. The explorer still wants the short words the
//! goldens were recorded with.

use crewmux_api::{ReadinessCode, RuntimeKind, SeatState};

/// Explorer wording for a seat lifecycle.
pub(crate) trait SeatBits {
    /// Short row label.
    fn label(&self) -> &'static str;
    /// Whether `a` may attach this seat.
    fn can_attach(&self) -> bool;
    /// Attention reason, when this state carries one.
    fn attention_code(&self) -> Option<&ReadinessCode>;
}

impl SeatBits for SeatState {
    fn label(&self) -> &'static str {
        self.as_str()
    }

    fn can_attach(&self) -> bool {
        matches!(self, Self::Ready | Self::Attention(_) | Self::Launching)
    }

    fn attention_code(&self) -> Option<&ReadinessCode> {
        self.code()
    }
}

/// Detail-pane wording for a runtime.
pub(crate) trait RuntimeBits {
    /// Short runtime name.
    fn label(self) -> &'static str;
}

impl RuntimeBits for RuntimeKind {
    fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::Terminal => "terminal",
            Self::Stub => "stub",
        }
    }
}
