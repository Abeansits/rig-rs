//! AC1 and AC7 against the real `rig` binary. Both sides are rig-rs (`--self`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

use xtask::diff::{
    CaseOutcome, DiffVerdict, execute_rs, exit_code, format_report, load_suite, self_reports,
    workspace_root,
};

#[test]
fn diff_selftest() {
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_rig"));
    let real_home = PathBuf::from(std::env::var("HOME").unwrap());
    let home_mtime = mtime(&real_home).unwrap();
    let conf = real_home.join(".tmux.conf");
    let conf_mtime = conf.metadata().ok().and_then(|meta| meta.modified().ok());
    let tmux_before = default_tmux_ls();

    let version = load_suite(&exceptions())
        .unwrap()
        .into_iter()
        .find(|case| case.case.id == "version");
    let version = version.unwrap();
    let left = execute_rs(&version, &bin).unwrap();
    let right = execute_rs(&version, &bin).unwrap();
    assert_eq!(left.canonical, right.canonical);
    assert_ne!(left.home, right.home);
    assert_ne!(left.tmux_socket, right.tmux_socket);
    assert!(left.home.starts_with(std::env::temp_dir()));
    assert_ne!(left.home, real_home);

    assert_eq!(mtime(&real_home).unwrap(), home_mtime);
    assert_eq!(
        conf.metadata().ok().and_then(|meta| meta.modified().ok()),
        conf_mtime
    );
    assert_eq!(default_tmux_ls(), tmux_before);
}

#[test]
fn initial_cases_are_equal_or_listed_as_ignored() {
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_rig"));
    let cases = load_suite(&exceptions()).unwrap();
    let scenario = cases
        .iter()
        .find(|case| case.case.id == "scenario-runner-probe")
        .unwrap();
    assert_eq!(
        scenario.scenario.as_deref(),
        Some("packages/test-system/scenarios/clean-lifecycle-no-residue.yaml")
    );
    let reports = self_reports(&cases, &bin).unwrap();
    let text = format_report(&reports);
    assert!(
        text.contains("daemon-health") && text.contains("needs UDS listener"),
        "{text}"
    );
    assert!(text.contains("| stub-seat | Equal |"), "{text}");
    assert!(text.contains("| up-ps-down-stub | Equal |"), "{text}");
    assert!(text.contains("| capture-stub | Equal |"), "{text}");
    assert!(
        text.contains("deferred until the daemon paste command exists"),
        "{text}"
    );
    for report in &reports {
        match &report.outcome {
            CaseOutcome::Verdict(DiffVerdict::Equal) | CaseOutcome::Ignored { .. } => {}
            CaseOutcome::Verdict(DiffVerdict::ExpectedDiff(_) | DiffVerdict::Unexpected(_))
            | CaseOutcome::Stale(_) => panic!("{} {:?}", report.id, report.outcome),
        }
    }
    assert_eq!(exit_code(&reports), 0);
}

#[test]
#[ignore = "needs UDS listener"]
fn daemon_health_against_product_daemon() {
    panic!("the product daemon does not listen on the Unix socket yet");
}

#[test]
fn stub_seat_against_product_cli() {
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_rig"));
    let cases = load_suite(&exceptions()).unwrap();
    let case = cases
        .into_iter()
        .find(|case| case.case.id == "stub-seat")
        .unwrap();
    assert!(case.needs.is_none(), "stub-seat is still ignored");
    let left = execute_rs(&case, &bin).unwrap();
    let right = execute_rs(&case, &bin).unwrap();
    assert_eq!(left.canonical, right.canonical);
}

fn exceptions() -> PathBuf {
    workspace_root().join("design/parity-exceptions.md")
}

fn mtime(path: &Path) -> std::io::Result<SystemTime> {
    fs::metadata(path)?.modified()
}

fn default_tmux_ls() -> Option<(i32, String, String)> {
    let path = std::env::var_os("PATH")?;
    let found = std::env::split_paths(&path).any(|dir| dir.join("tmux").is_file());
    if !found {
        return None;
    }
    let mut command = Command::new("tmux");
    command
        .arg("ls")
        .env_remove("TMUX")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command.spawn().ok()?;
    let output = child.wait_with_output().ok()?;
    Some((
        output.status.code().unwrap_or(1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ))
}
