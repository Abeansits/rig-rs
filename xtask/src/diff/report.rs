//! Human-readable diff report, including ignored cases.

use std::fmt::Write;

use super::types::DiffVerdict;
use crate::error::Error;

/// One row of `xtask diff`.
#[derive(Debug)]
pub enum CaseOutcome {
    /// The two sides were judged.
    Verdict(DiffVerdict),
    /// The case is waiting on a later slice.
    Ignored {
        /// Text after `needs`, such as `slice 0`.
        reason: String,
    },
    /// The case was marked expected-diff but the sides matched.
    Stale(Error),
}

/// A finished or skipped case.
#[derive(Debug)]
pub struct CaseReport {
    /// Case id.
    pub id: String,
    /// What happened.
    pub outcome: CaseOutcome,
}

/// Markdown report. Ignored cases are listed so slice gates stay visible.
#[must_use]
pub fn format_report(cases: &[CaseReport]) -> String {
    let mut out = String::from("# diff report\n\n| case | result |\n|---|---|\n");
    for case in cases {
        let result = match &case.outcome {
            CaseOutcome::Verdict(DiffVerdict::Equal) => "Equal".to_owned(),
            CaseOutcome::Verdict(DiffVerdict::ExpectedDiff(pe)) => format!("ExpectedDiff({pe})"),
            CaseOutcome::Verdict(DiffVerdict::Unexpected(_)) => "Unexpected".to_owned(),
            CaseOutcome::Ignored { reason } => format!("ignored: needs {reason}"),
            CaseOutcome::Stale(error) => error.to_string(),
        };
        let _ = writeln!(out, "| {} | {result} |", case.id);
    }
    out.push_str("\n## ignored\n\n");
    let mut any = false;
    for case in cases {
        if let CaseOutcome::Ignored { reason } = &case.outcome {
            let _ = writeln!(out, "- {}: needs {reason}", case.id);
            any = true;
        }
    }
    if !any {
        out.push_str("- (none)\n");
    }
    out.push_str("\n## failures\n\n");
    let mut failed = false;
    for case in cases {
        match &case.outcome {
            CaseOutcome::Verdict(DiffVerdict::Unexpected(diff)) => {
                failed = true;
                let _ = write!(out, "### {}\n\n```diff\n{diff}```\n", case.id);
            }
            CaseOutcome::Stale(error) => {
                failed = true;
                let _ = write!(out, "### {}\n\n{error}\n", case.id);
            }
            CaseOutcome::Verdict(DiffVerdict::Equal | DiffVerdict::ExpectedDiff(_))
            | CaseOutcome::Ignored { .. } => {}
        }
    }
    if !failed {
        out.push_str("(none)\n");
    }
    out
}

/// Process status for a finished report.
///
/// `0` when every verdict is Equal or `ExpectedDiff` (ignored cases included).
/// `1` when any verdict is Unexpected or a stale exception was reported.
#[must_use]
pub fn exit_code(cases: &[CaseReport]) -> i32 {
    let bad = cases.iter().any(|case| {
        matches!(
            case.outcome,
            CaseOutcome::Verdict(DiffVerdict::Unexpected(_)) | CaseOutcome::Stale(_)
        )
    });
    i32::from(bad)
}
