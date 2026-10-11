//! Run a suite in self mode or against TypeScript v0.6.9.

use std::path::Path;

use super::compare::judge;
use super::golden::{read_ts_golden, write_ts_golden};
use super::report::{CaseOutcome, CaseReport};
use super::run::{execute, execute_rs};
use super::types::{LoadedCase, Side, workspace_root};
use crate::error::Error;

/// Both sides are the same rig-rs binary. Goldens are not read or written.
///
/// # Errors
///
/// Returns [`Error`] when a case fails to spawn. A stale exception is a row, not an error.
pub fn self_reports(cases: &[LoadedCase], bin: &Path) -> Result<Vec<CaseReport>, Error> {
    let mut reports = Vec::new();
    for case in cases {
        reports.push(self_one(case, bin)?);
    }
    Ok(reports)
}

fn self_one(case: &LoadedCase, bin: &Path) -> Result<CaseReport, Error> {
    if let Some(reason) = &case.needs {
        return Ok(ignored(&case.case.id, reason));
    }
    let left = execute_rs(case, bin)?;
    let right = execute_rs(case, bin)?;
    // expected_diff names a TypeScript-versus-rig difference. Both sides here are
    // rig-rs, so a match is Equal. Stale exceptions are reported only by ts_reports.
    from_judge(
        &case.case.id,
        judge(&case.case.id, &left.canonical, &right.canonical, None),
    )
}

/// Run TypeScript, pin or check its golden, then compare rig-rs to that golden.
///
/// `--update` rewrites only `*.ts.snap`. There is no rig-rs accept path.
///
/// # Errors
///
/// Returns [`Error`] when TypeScript cannot be launched or a golden is missing
/// and `update` is false.
pub fn ts_reports(
    cases: &[LoadedCase],
    bin: &Path,
    node: &Path,
    pkg: &Path,
    update: bool,
) -> Result<Vec<CaseReport>, Error> {
    let snaps = workspace_root().join("tests/diff/snapshots");
    let mut reports = Vec::new();
    for case in cases {
        if let Some(reason) = &case.needs {
            reports.push(ignored(&case.case.id, reason));
            continue;
        }
        let ts_side = Side::Ts {
            node: node.to_path_buf(),
            pkg: pkg.to_path_buf(),
        };
        let ts = execute(case, &ts_side, None)?;
        if update {
            write_ts_golden(&snaps, &case.case.id, &ts.canonical)?;
        } else {
            let golden = read_ts_golden(&snaps, &case.case.id)?;
            if golden != ts.canonical {
                let verdict = judge(&case.case.id, &golden, &ts.canonical, None)?;
                reports.push(CaseReport {
                    id: case.case.id.clone(),
                    outcome: CaseOutcome::Verdict(verdict),
                });
                continue;
            }
        }
        let rs = execute_rs(case, bin)?;
        let verdict = judge(
            &case.case.id,
            &ts.canonical,
            &rs.canonical,
            case.case.expected_diff.as_ref(),
        );
        reports.push(from_judge(&case.case.id, verdict)?);
    }
    Ok(reports)
}

fn ignored(id: &str, reason: &str) -> CaseReport {
    CaseReport {
        id: id.to_owned(),
        outcome: CaseOutcome::Ignored {
            reason: reason.to_owned(),
        },
    }
}

fn from_judge(
    id: &str,
    judged: Result<super::types::DiffVerdict, Error>,
) -> Result<CaseReport, Error> {
    let outcome = match judged {
        Ok(verdict) => CaseOutcome::Verdict(verdict),
        Err(error @ Error::Stale { .. }) => CaseOutcome::Stale(error),
        Err(error) => return Err(error),
    };
    Ok(CaseReport {
        id: id.to_owned(),
        outcome,
    })
}
