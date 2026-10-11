//! Ledger rows. One row per TypeScript test file, plus optional case rows
//! whose `ts_path` is `<file> > <case name>`.

use crate::diff::PeId;
use crate::error::Error;

/// `todo`, `ported`, `reused`, `waived`, or `diverged`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerStatus {
    /// Not yet assigned to a Rust test.
    Todo,
    /// Covered by the named Rust test.
    Ported,
    /// The Rust test already existed and covers this file.
    Reused,
    /// Out of scope, with a note.
    Waived,
    /// Behaviour differs on purpose. Requires a parity-exception id.
    Diverged,
}

impl LedgerStatus {
    /// Lowercase CSV token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::Ported => "ported",
            Self::Reused => "reused",
            Self::Waived => "waived",
            Self::Diverged => "diverged",
        }
    }

    pub(crate) fn parse(text: &str) -> Result<Self, ()> {
        match text {
            "todo" => Ok(Self::Todo),
            "ported" => Ok(Self::Ported),
            "reused" => Ok(Self::Reused),
            "waived" => Ok(Self::Waived),
            "diverged" => Ok(Self::Diverged),
            _ => Err(()),
        }
    }
}

/// One CSV row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerRow {
    /// Path relative to the `OpenRig` checkout, using `/`.
    ///
    /// A case row is `<ts_path> > <case name>` and names one `#[test]` inside that file.
    pub ts_path: String,
    /// Slice id, when the row has been assigned.
    pub slice: Option<String>,
    /// Lifecycle of the port.
    pub status: LedgerStatus,
    /// Rust test name as printed by `cargo test -- --list`.
    pub rust_test: Option<String>,
    /// Required when `status` is [`LedgerStatus::Diverged`].
    pub pe_id: Option<PeId>,
    /// Required when `status` is [`LedgerStatus::Waived`].
    pub note: String,
}

/// CSV header, in order.
pub const COLUMNS: [&str; 6] = ["ts_path", "slice", "status", "rust_test", "pe_id", "note"];

pub(crate) fn optional(text: &str) -> Option<String> {
    if text.is_empty() {
        None
    } else {
        Some(text.to_owned())
    }
}

pub(crate) fn parse_pe(text: &str) -> Result<Option<PeId>, Error> {
    if text.is_empty() {
        Ok(None)
    } else {
        PeId::parse(text).map(Some)
    }
}
