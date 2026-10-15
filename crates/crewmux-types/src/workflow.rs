//! Workflow spec, role, step, and instance ids.
//!
//! Spec and role names are caller-supplied tokens, not ULIDs. Instance ids are
//! ULIDs. An empty token is rejected by [`NameId::new`]; the spec parser uses
//! [`NameId::raw`] so a missing field can still reach the validator.

use std::fmt;

use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::error::TypeError;

/// A non-empty workflow name token.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NameId(String);

impl NameId {
    /// Reject an empty token or one that contains whitespace.
    ///
    /// # Errors
    ///
    /// Returns [`TypeError::Name`] when `text` is empty or has whitespace or a control character.
    pub fn new(text: impl Into<String>) -> Result<Self, TypeError> {
        let text = text.into();
        let ok = !text.is_empty() && !text.chars().any(|ch| ch.is_whitespace() || ch.is_control());
        if ok {
            Ok(Self(text))
        } else {
            Err(TypeError::Name(text))
        }
    }

    /// Keep `text` as parsed, including an empty field the validator must report.
    #[must_use]
    pub fn raw(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// Borrow the token.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NameId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Workflow spec id (`linear-build`).
pub type SpecId = NameId;
/// Role name declared in `workflow.roles`.
pub type RoleId = NameId;
/// Step id declared in `workflow.steps`.
pub type StepId = NameId;

/// A workflow instance id. The wire form is a ULID.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkflowInstanceId(String);

impl WorkflowInstanceId {
    /// A new random ULID.
    #[must_use]
    pub fn generate() -> Self {
        Self(Ulid::generate().to_string())
    }

    /// Reject an empty id.
    ///
    /// # Errors
    ///
    /// Returns [`TypeError::Name`] when `text` is empty or has whitespace.
    pub fn new(text: impl Into<String>) -> Result<Self, TypeError> {
        let text = text.into();
        let ok = !text.is_empty() && !text.chars().any(|ch| ch.is_whitespace() || ch.is_control());
        if ok {
            Ok(Self(text))
        } else {
            Err(TypeError::Name(text))
        }
    }

    /// Borrow the id.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WorkflowInstanceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
