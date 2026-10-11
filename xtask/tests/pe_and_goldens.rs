//! PE-id lookup, stale exceptions, goldens, and process exit codes.

use std::fs;

use xtask::Error;
use xtask::diff::{
    CaseOutcome, CaseReport, DiffVerdict, Observation, PeId, canonical, exit_code, judge,
    load_case, pe_ids_in, read_ts_golden, unified_diff, write_ts_golden,
};

fn scratch(name: &str) -> std::io::Result<tempfile::TempDir> {
    tempfile::TempDir::with_prefix(name)
}

#[test]
fn fixture_copy_accepts_pe_test_and_rejects_an_unknown_id() {
    let dir = scratch("pe-").unwrap();
    let markdown = dir.path().join("parity-exceptions.md");
    fs::write(&markdown, "# fixture\n\n## PE-001: real\n\n## PE-TEST\n").unwrap();
    let ids = pe_ids_in(&fs::read_to_string(&markdown).unwrap());
    assert!(ids.iter().any(|id| id.as_str() == "PE-TEST"));
    let case_path = dir.path().join("marked.json");
    fs::write(
        &case_path,
        r#"{"id":"marked","expected_diff":"PE-TEST","steps":[{"op":"cli","argv":["--version"]}]}"#,
    )
    .unwrap();
    let loaded = load_case(&case_path, &markdown).unwrap();
    assert_eq!(loaded.case.expected_diff.unwrap().as_str(), "PE-TEST");
    fs::write(
        &case_path,
        r#"{"id":"marked","expected_diff":"PE-NOPE","steps":[{"op":"cli","argv":["--version"]}]}"#,
    )
    .unwrap();
    let error = load_case(&case_path, &markdown).unwrap_err();
    assert!(matches!(error, Error::UnknownPe { .. }), "{error}");
}

#[test]
fn equal_outputs_on_an_expected_diff_are_a_stale_exception() {
    let pe = PeId::parse("PE-TEST").unwrap();
    let error = judge("version", "same\n", "same\n", Some(&pe)).unwrap_err();
    assert!(error.to_string().contains("stale exception"));
    let reports = [CaseReport {
        id: "version".to_owned(),
        outcome: CaseOutcome::Stale(error),
    }];
    assert_eq!(exit_code(&reports), 1);
}

#[test]
fn a_real_difference_with_a_pe_id_is_expected_diff() {
    let pe = PeId::parse("PE-001").unwrap();
    let verdict = judge("version", "left\n", "right\n", Some(&pe)).unwrap();
    assert_eq!(verdict, DiffVerdict::ExpectedDiff(pe));
}

#[test]
fn unexpected_includes_a_unified_diff_and_exits_1() {
    let verdict = judge("version", "left\n", "right\n", None).unwrap();
    let DiffVerdict::Unexpected(diff) = verdict else {
        panic!("expected unexpected")
    };
    assert!(diff.contains("--- ts"));
    assert!(diff.contains("+++ rs"));
    assert!(diff.contains("-left"));
    assert!(diff.contains("+right"));
    assert_eq!(diff, unified_diff("left\n", "right\n"));
    let reports = [CaseReport {
        id: "version".to_owned(),
        outcome: CaseOutcome::Verdict(DiffVerdict::Unexpected(diff)),
    }];
    assert_eq!(exit_code(&reports), 1);
}

#[test]
fn equal_and_ignored_exit_0() {
    let reports = [
        CaseReport {
            id: "version".to_owned(),
            outcome: CaseOutcome::Verdict(DiffVerdict::Equal),
        },
        CaseReport {
            id: "daemon-health".to_owned(),
            outcome: CaseOutcome::Ignored {
                reason: "slice 0".to_owned(),
            },
        },
    ];
    assert_eq!(exit_code(&reports), 0);
}

#[test]
fn update_writes_only_a_ts_snapshot() {
    let dir = scratch("snap-").unwrap();
    write_ts_golden(dir.path(), "version", "{\n}\n").unwrap();
    let names: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["version.ts.snap".to_owned()]);
    assert_eq!(read_ts_golden(dir.path(), "version").unwrap(), "{\n}\n");
    let error = write_ts_golden(dir.path(), "../escape", "x").unwrap_err();
    assert!(error.to_string().contains("snapshot"));
}

#[test]
fn canonical_observation_matches_the_insta_snapshot() {
    let obs = Observation {
        stdout: "ok".to_owned(),
        stderr: String::new(),
        exit: 0,
        json: None,
        files: std::collections::BTreeMap::new(),
        tmux_argv: vec![vec!["tmux".to_owned(), "-V".to_owned()]],
    };
    let text = canonical(&obs).unwrap();
    insta::assert_snapshot!(text);
}
