//! Doctor check DTOs. Status names match the TypeScript `pass | warn | fail | skipped` set.

use serde::{Deserialize, Serialize};

/// Outcome of one `rig doctor` check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    /// The check succeeded.
    Pass,
    /// The check found something the operator should look at. It does not fail the process.
    Warn,
    /// The check failed. The process exits 1 when any check has this status.
    Fail,
    /// The check does not apply to this invocation.
    Skipped,
}

/// One row of `rig doctor --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorCheck {
    /// Stable check name from the slice 7 table.
    pub name: &'static str,
    /// Pass, warn, fail, or skipped.
    pub status: CheckStatus,
    /// Human-readable result. Versions are included where the check reports one.
    pub message: String,
    /// Present for every warn and fail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

impl DoctorCheck {
    /// A passing check with no fix hint.
    #[must_use]
    pub fn pass(name: &'static str, message: impl Into<String>) -> Self {
        Self {
            name,
            status: CheckStatus::Pass,
            message: message.into(),
            fix: None,
        }
    }

    /// A warning. `fix` is required by the doctor contract.
    #[must_use]
    pub fn warn(name: &'static str, message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            name,
            status: CheckStatus::Warn,
            message: message.into(),
            fix: Some(fix.into()),
        }
    }

    /// A failure. `fix` is required by the doctor contract.
    #[must_use]
    pub fn fail(name: &'static str, message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            name,
            status: CheckStatus::Fail,
            message: message.into(),
            fix: Some(fix.into()),
        }
    }

    /// A check that was not run.
    #[must_use]
    pub fn skipped(name: &'static str, message: impl Into<String>) -> Self {
        Self {
            name,
            status: CheckStatus::Skipped,
            message: message.into(),
            fix: None,
        }
    }
}
