//! Issue codes, fields, and messages for every static TypeScript workflow fixture.
//!
//! The golden is the pinned v0.6.9 parser and validator output for each
//! `workflow:` template and builtin spec. A mismatch here is a parity break.

use crewmux_orchestration::{Error, Runtime, Severity, Validation};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    name: String,
    path: String,
    yaml: String,
    ok: Option<bool>,
    #[serde(default)]
    issues: Vec<ExpectedIssue>,
    parse: Option<ExpectedParse>,
}

#[derive(Deserialize)]
struct ExpectedIssue {
    code: String,
    field: Option<String>,
    message: String,
    severity: String,
}

#[derive(Deserialize)]
struct ExpectedParse {
    code: String,
    message: String,
}

#[test]
fn issue_codes_match_the_typescript_fixtures() {
    let rows: Vec<Fixture> =
        serde_json::from_str(include_str!("../fixtures/issue-matrix.json")).expect("matrix");
    assert!(rows.len() > 100, "{}", rows.len());
    for row in &rows {
        if let Some(diff) = compare(row) {
            panic!("{diff}");
        }
    }
}

fn compare(row: &Fixture) -> Option<String> {
    match Runtime::validate_text(&row.yaml, &row.path) {
        Ok(report) => report_diff(row, &report),
        Err(Error::Rejected { code, message, .. }) => parse_diff(row, code, &message),
        Err(err) => Some(format!("{}: {err}", row.name)),
    }
}

fn report_diff(row: &Fixture, report: &Validation) -> Option<String> {
    if row.parse.is_some() {
        return Some(format!(
            "{} parsed in TypeScript and failed in Rust",
            row.name
        ));
    }
    if report.ok != row.ok.unwrap_or(false) {
        return Some(format!("{} ok {} vs {:?}", row.name, report.ok, row.ok));
    }
    if report.issues.len() != row.issues.len() {
        let codes: Vec<&str> = report.issues.iter().map(|issue| issue.code).collect();
        return Some(format!("{} codes {codes:?}", row.name));
    }
    for (index, expected) in row.issues.iter().enumerate() {
        let Some(issue) = report.issues.get(index) else {
            return Some(format!("{} missing issue {index}", row.name));
        };
        let severity = match issue.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        if issue.code != expected.code
            || issue.field.as_deref() != expected.field.as_deref()
            || issue.message != expected.message
            || severity != expected.severity
        {
            return Some(format!(
                "{} issue {index}: {} {:?} {} != {} {:?} {}",
                row.name,
                issue.code,
                issue.field,
                issue.message,
                expected.code,
                expected.field,
                expected.message
            ));
        }
    }
    None
}

fn parse_diff(row: &Fixture, code: &str, message: &str) -> Option<String> {
    let Some(expected) = &row.parse else {
        return Some(format!(
            "{} validated in TypeScript but Rust rejected it: {code}: {message}",
            row.name
        ));
    };
    if code != expected.code || message != expected.message {
        return Some(format!(
            "{} parse {code}: {message} != {}: {}",
            row.name, expected.code, expected.message
        ));
    }
    None
}
