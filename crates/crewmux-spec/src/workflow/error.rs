//! Parse failures. Codes match `WorkflowSpecError` in the TypeScript spec cache.

use thiserror::Error;

/// A workflow YAML document that failed the closed-key parse.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("{message}")]
pub struct WorkflowParseError {
    /// Stable code such as `spec_unknown_key`.
    pub code: &'static str,
    /// Operator text, including the TypeScript sentence.
    pub message: String,
    /// Field path when the TypeScript error carries one.
    pub field: Option<String>,
}

impl WorkflowParseError {
    pub(crate) fn new(
        code: &'static str,
        message: impl Into<String>,
        field: Option<String>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            field,
        }
    }
}
