//! `xtask ledger check` rules.

use std::collections::{BTreeMap, BTreeSet};

use super::types::{LedgerRow, LedgerStatus};
use crate::diff::PeId;

/// Problems with `rows`. Empty means the ledger is valid.
///
/// `listed_tests` is the set printed by `cargo test -- --list`. When it is
/// `None`, rows that name a Rust test are not checked against the binary list
/// (the caller skipped the cargo invocation because no row names one).
#[must_use]
pub fn validate(
    rows: &[LedgerRow],
    pe_ids: &BTreeSet<PeId>,
    listed_tests: Option<&BTreeSet<String>>,
) -> Vec<String> {
    let mut errors = Vec::new();
    let mut seen = BTreeMap::<String, usize>::new();
    for (index, row) in rows.iter().enumerate() {
        let line_no = index.saturating_add(2);
        if let Some(first) = seen.insert(row.ts_path.clone(), line_no) {
            errors.push(format!(
                "line {line_no}: duplicate ts_path {} (also on line {first})",
                row.ts_path
            ));
        }
        check_status(row, line_no, pe_ids, listed_tests, &mut errors);
    }
    check_case_rows(rows, &mut errors);
    errors
}

fn check_case_rows(rows: &[LedgerRow], errors: &mut Vec<String>) {
    let files: BTreeSet<&str> = rows
        .iter()
        .filter(|row| !row.ts_path.contains('>'))
        .map(|row| row.ts_path.as_str())
        .collect();
    for (index, row) in rows.iter().enumerate() {
        if !row.ts_path.contains('>') {
            continue;
        }
        let line_no = index.saturating_add(2);
        let Some((parent, _)) = split_case(&row.ts_path) else {
            errors.push(format!(
                "line {line_no}: case row {} must look like `<ts_path> > <case name>`",
                row.ts_path
            ));
            continue;
        };
        if !files.contains(parent) {
            errors.push(format!(
                "line {line_no}: case row parent {parent} is not a file-level ledger row"
            ));
        }
        if !matches!(row.status, LedgerStatus::Ported | LedgerStatus::Reused) {
            errors.push(format!(
                "line {line_no}: case row {} must be ported or reused",
                row.ts_path
            ));
        }
        if row.rust_test.as_deref().unwrap_or("").is_empty() {
            errors.push(format!(
                "line {line_no}: case row {} requires rust_test",
                row.ts_path
            ));
        }
    }
}

fn split_case(ts_path: &str) -> Option<(&str, &str)> {
    let (parent, case_name) = ts_path.split_once(" > ")?;
    if parent.is_empty() || case_name.is_empty() || case_name.contains(" > ") {
        return None;
    }
    Some((parent, case_name))
}

fn check_status(
    row: &LedgerRow,
    line_no: usize,
    pe_ids: &BTreeSet<PeId>,
    listed_tests: Option<&BTreeSet<String>>,
    errors: &mut Vec<String>,
) {
    match row.status {
        LedgerStatus::Diverged => check_diverged(row, line_no, pe_ids, errors),
        LedgerStatus::Ported | LedgerStatus::Reused => {
            if row.rust_test.as_deref().unwrap_or("").is_empty() {
                errors.push(format!(
                    "line {line_no}: {} {} requires rust_test",
                    row.status.as_str(),
                    row.ts_path
                ));
            }
        }
        LedgerStatus::Waived => {
            if row.note.trim().is_empty() {
                errors.push(format!(
                    "line {line_no}: waived {} requires a note",
                    row.ts_path
                ));
            }
        }
        LedgerStatus::Todo => {}
    }
    if let (Some(tests), Some(name)) = (listed_tests, row.rust_test.as_deref())
        && !name.is_empty()
        && !tests.contains(name)
    {
        errors.push(format!(
            "line {line_no}: rust_test {name} is not in `cargo test -- --list`"
        ));
    }
}

fn check_diverged(
    row: &LedgerRow,
    line_no: usize,
    pe_ids: &BTreeSet<PeId>,
    errors: &mut Vec<String>,
) {
    match &row.pe_id {
        None => errors.push(format!(
            "line {line_no}: diverged {} requires a pe_id",
            row.ts_path
        )),
        Some(pe) if !pe_ids.contains(pe) => errors.push(format!(
            "line {line_no}: diverged {} has unknown pe_id {pe}",
            row.ts_path
        )),
        Some(_) => {}
    }
}

/// Test names from `cargo test -- --list` output.
#[must_use]
pub fn parse_test_list(output: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for line in output.lines() {
        let Some(name) = line.strip_suffix(": test") else {
            continue;
        };
        let name = name.trim();
        if !name.is_empty() && !name.contains(' ') {
            names.insert(name.to_owned());
        }
    }
    names
}
