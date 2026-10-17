//! A composed context pack handed to the guarded paste path.

/// Paste payload for one composed pack. The daemon sends `body` through the
/// slice 4 guard. This type does not touch a pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextPaste {
    /// Plain composed text.
    pub body: String,
    /// Token estimate of `body`.
    pub tokens_est: u32,
    /// Pack refs that contributed bytes.
    pub sources: Vec<String>,
}
