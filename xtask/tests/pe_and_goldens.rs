//! PE-id lookup, stale exceptions, goldens, and process exit codes.

use std::fs;

use xtask::Error;
use xtask::diff::{
    CaseOutcome, CaseReport, DiffVerdict, ExpectedScope, ObsField, Observation, PeId, canonical,
    covers_for, exit_code, judge, load_case, pe_ids_in, read_rs_golden, read_ts_golden,
    unified_diff, workspace_root, write_ts_golden,
};

fn scratch(name: &str) -> std::io::Result<tempfile::TempDir> {
    tempfile::TempDir::with_prefix(name)
}

#[test]
fn fixture_copy_accepts_pe_test_and_rejects_an_unknown_id() {
    let dir = scratch("pe-").unwrap();
    let markdown = dir.path().join("parity-exceptions.md");
    fs::write(
        &markdown,
        "# fixture\n\n## PE-001: real\n\n## PE-TEST\n\n- **Covers:** stdout\n",
    )
    .unwrap();
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
    assert_eq!(loaded.case.covers, vec![ObsField::Stdout]);
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
    let same = observation("0.6.9\n", "", 0, vec![argv(&["tmux", "-V"])]).unwrap();
    let error = judge(
        "scenario-runner-probe",
        &same,
        &same,
        Some(&scope(&same).unwrap()),
    )
    .unwrap_err();
    assert!(error.to_string().contains("stale exception"));
    assert!(error.to_string().contains("outputs are equal"));
    let reports = [CaseReport {
        id: "scenario-runner-probe".to_owned(),
        outcome: CaseOutcome::Stale(error),
    }];
    assert_eq!(exit_code(&reports), 1);
}

#[test]
fn a_real_difference_with_a_pe_id_is_expected_diff() {
    let ts = observation("0.6.9\n", "", 0, vec![argv(&["tmux", "-V"])]).unwrap();
    let rs = observation("", "", 0, vec![argv(&["tmux", "-V"])]).unwrap();
    let scope = scope(&rs).unwrap();
    let verdict = judge("scenario-runner-probe", &ts, &rs, Some(&scope)).unwrap();
    assert_eq!(verdict, DiffVerdict::ExpectedDiff(scope.pe));
}

#[test]
fn excepted_exit_code_is_unexpected() {
    let ts = observation("add\n", "", 0, Vec::new()).unwrap();
    let rs = observation("", "", 101, Vec::new()).unwrap();
    let snap = observation("", "", 0, Vec::new()).unwrap();
    let verdict = judge("help-top", &ts, &rs, Some(&scope(&snap).unwrap())).unwrap();
    let DiffVerdict::Unexpected(diff) = verdict else {
        panic!("expected unexpected, got {verdict:?}")
    };
    assert!(diff.contains("101"), "{diff}");
}

#[test]
fn excepted_stderr_panic_is_unexpected() {
    let panic = "thread 'main' panicked at src/main.rs:1:1:\nexplicit panic\n";
    let ts = observation("0.6.9\n", "", 0, vec![argv(&["tmux", "-V"])]).unwrap();
    let rs = observation("", panic, 0, vec![argv(&["tmux", "-V"])]).unwrap();
    let snap = observation("", "", 0, vec![argv(&["tmux", "-V"])]).unwrap();
    let verdict = judge(
        "scenario-runner-probe",
        &ts,
        &rs,
        Some(&scope(&snap).unwrap()),
    )
    .unwrap();
    let DiffVerdict::Unexpected(diff) = verdict else {
        panic!("expected unexpected, got {verdict:?}")
    };
    assert!(diff.contains("panicked"), "{diff}");
}

#[test]
fn excepted_extra_tmux_call_is_unexpected() {
    let ts = observation("0.6.9\n", "", 0, vec![argv(&["tmux", "-V"])]).unwrap();
    let rs = observation(
        "",
        "",
        0,
        vec![argv(&["tmux", "-V"]), argv(&["tmux", "kill-server"])],
    )
    .unwrap();
    let snap = observation("", "", 0, vec![argv(&["tmux", "-V"])]).unwrap();
    let verdict = judge(
        "scenario-runner-probe",
        &ts,
        &rs,
        Some(&scope(&snap).unwrap()),
    )
    .unwrap();
    let DiffVerdict::Unexpected(diff) = verdict else {
        panic!("expected unexpected, got {verdict:?}")
    };
    assert!(diff.contains("kill-server"), "{diff}");
}

#[test]
fn changed_rust_snapshot_is_stale() {
    let ts = observation("0.6.9\n", "", 0, vec![argv(&["tmux", "-V"])]).unwrap();
    let rs = observation("", "", 0, vec![argv(&["tmux", "-V"])]).unwrap();
    let changed = observation("partial\n", "", 0, vec![argv(&["tmux", "-V"])]).unwrap();
    let error = judge(
        "scenario-runner-probe",
        &ts,
        &rs,
        Some(&scope(&changed).unwrap()),
    )
    .unwrap_err();
    let text = error.to_string();
    assert!(text.contains("stale exception"), "{text}");
    assert!(text.contains("no longer matches"), "{text}");
    let reports = [CaseReport {
        id: "scenario-runner-probe".to_owned(),
        outcome: CaseOutcome::Stale(error),
    }];
    assert_eq!(exit_code(&reports), 1);
}

#[test]
fn pe_003_covers_stdout_only() {
    let text = fs::read_to_string(workspace_root().join("design/parity-exceptions.md")).unwrap();
    let covers = covers_for(&text, &PeId::parse("PE-003").unwrap()).unwrap();
    assert_eq!(covers, vec![ObsField::Stdout]);
}

#[test]
fn pe_006_covers_stdout_and_exit() {
    let text = fs::read_to_string(workspace_root().join("design/parity-exceptions.md")).unwrap();
    let covers = covers_for(&text, &PeId::parse("PE-006").unwrap()).unwrap();
    assert_eq!(covers, vec![ObsField::Stdout, ObsField::Exit]);
}

#[test]
fn expected_diff_without_covers_fails_to_load() {
    let dir = scratch("pe-bare-").unwrap();
    let markdown = dir.path().join("parity-exceptions.md");
    fs::write(&markdown, "## PE-BARE: no fields\n").unwrap();
    let case_path = dir.path().join("bare.json");
    fs::write(
        &case_path,
        r#"{"id":"bare","expected_diff":"PE-BARE","steps":[{"op":"cli","argv":["--version"]}]}"#,
    )
    .unwrap();
    let error = load_case(&case_path, &markdown).unwrap_err();
    assert!(error.to_string().contains("does not declare"), "{error}");
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
fn committed_rust_snapshots_match_the_slice1_cli() {
    let dir = workspace_root().join("tests/diff/snapshots");
    let help = observation("attach\ncapture\ndown\nhelp\nps\nup", "", 0, Vec::new()).unwrap();
    let version = observation("rig 0.0.0\n", "", 0, vec![argv(&["tmux", "-V"])]).unwrap();
    for (id, expected) in [("help-top", help), ("scenario-runner-probe", version)] {
        let snap = read_rs_golden(&dir, id).unwrap();
        assert_eq!(snap, expected, "{id}");
        let ts = read_ts_golden(&dir, id).unwrap();
        assert_ne!(ts, snap, "{id}");
    }
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

fn observation(
    stdout: &str,
    stderr: &str,
    exit: i32,
    tmux_argv: Vec<Vec<String>>,
) -> Result<String, Error> {
    canonical(&Observation {
        stdout: stdout.to_owned(),
        stderr: stderr.to_owned(),
        exit,
        json: None,
        files: std::collections::BTreeMap::new(),
        tmux_argv,
    })
}

fn scope(snapshot: &str) -> Result<ExpectedScope, Error> {
    Ok(ExpectedScope {
        pe: PeId::parse("PE-003")?,
        covers: vec![ObsField::Stdout],
        rust_snapshot: snapshot.to_owned(),
    })
}

fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_owned()).collect()
}
