//! Manifest parse failures. Codes match the TypeScript `ContextPackError` names.

use thiserror::Error;

/// Why a context-pack manifest was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManifestCode {
    /// The YAML parser rejected the document.
    Parse,
    /// The document parsed but failed the schema.
    Invalid,
}

impl ManifestCode {
    /// Stable code string used in scan errors.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Parse => "manifest_parse_error",
            Self::Invalid => "manifest_invalid",
        }
    }
}

/// A manifest that is not valid YAML or does not match the pack schema.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("{message}")]
pub struct ManifestError {
    /// Machine code.
    pub code: ManifestCode,
    /// TypeScript-compatible sentence.
    pub message: String,
}

impl ManifestError {
    pub(crate) fn new(code: ManifestCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}
