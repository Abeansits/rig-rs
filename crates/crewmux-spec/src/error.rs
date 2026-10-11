//! Spec parse and validation errors.

use thiserror::Error;

/// A `RigSpec` that could not be parsed or that failed validation.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("{message} at line {line} column {col} ({path})")]
pub struct SpecError {
    /// 1-based line. `1` when the parser had no location.
    pub line: u32,
    /// 1-based column. `1` when the parser had no location.
    pub col: u32,
    /// Field path such as `seats[0].runtime`. Empty when the error is the whole document.
    pub path: String,
    /// Human-readable reason.
    pub message: String,
}

impl SpecError {
    /// Build an error at a known location.
    #[must_use]
    pub fn at(line: u32, col: u32, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            line,
            col,
            path: path.into(),
            message: message.into(),
        }
    }
}
