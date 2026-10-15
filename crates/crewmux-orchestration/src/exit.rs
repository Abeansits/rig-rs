//! Recorded workflow exits.

/// The closed exit enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// Hand the step to the next owner.
    Handoff,
    /// Park on a blocker.
    Waiting,
    /// Finish the instance.
    Done,
    /// Fail the instance.
    Failed,
}

impl Exit {
    /// Wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Handoff => "handoff",
            Self::Waiting => "waiting",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    /// Parse a wire token.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "handoff" => Self::Handoff,
            "waiting" => Self::Waiting,
            "done" => Self::Done,
            "failed" => Self::Failed,
            _ => return None,
        })
    }
}
