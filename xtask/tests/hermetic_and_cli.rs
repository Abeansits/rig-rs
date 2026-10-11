//! Hermetic refusals, the tmux shim, and xtask process exit codes.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

use xtask::diff::{HermeticHome, NormRule, Observation, normalize_observation};

fn run(
    args: &[&str],
    home: Option<&Path>,
    remove: &[&str],
) -> std::io::Result<(i32, String, String)> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_xtask"));
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in remove {
        command.env_remove(name);
    }
    if let Some(home) = home {
        command.env("HOME", home);
    }
    let child = command.spawn()?;
    let output = child.wait_with_output()?;
    let code = output.status.code().unwrap_or(1);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Ok((code, stdout, stderr))
}

#[test]
fn foreign_daemon_clock_and_tmux_are_refused() {
    let path = ("PATH".to_owned(), "/usr/bin:/bin".to_owned());
    let daemon =
        HermeticHome::from_vars(&[path.clone(), ("OPENRIG_URL".into(), "http://x".into())]);
    assert!(
        daemon
            .unwrap_err()
            .to_string()
            .contains("ambient daemon target")
    );
    let clock =
        HermeticHome::from_vars(&[path.clone(), ("OPENRIG_TEST_CLOCK_NOW".into(), "1".into())]);
    assert!(clock.unwrap_err().to_string().contains("injected-clock"));
    let attached = HermeticHome::from_vars(&[path, ("TMUX".into(), "/tmp/default".into())]);
    assert!(
        attached
            .unwrap_err()
            .to_string()
            .contains("tmux attachment")
    );
}

#[test]
fn tmux_shim_is_first_on_path_and_records_argv() {
    let path = std::env::var("PATH").unwrap_or_default();
    let home = HermeticHome::from_vars(&[("PATH".into(), path)]).unwrap();
    let shim_dir = home.shim_path().parent().unwrap().display().to_string();
    let child_path = home.child_env().get("PATH").unwrap();
    assert!(child_path.starts_with(&shim_dir), "{child_path}");
    if home.can_probe_tmux() {
        assert_eq!(home.invoke_tmux(&["-V".into()]).unwrap(), 0);
        let _code = home
            .invoke_tmux(&["-L".into(), "other".into(), "ls".into()])
            .unwrap();
        let argv = home.tmux_argv().unwrap();
        assert_eq!(
            argv.first().map(Vec::as_slice),
            Some(["tmux".to_owned(), "-V".to_owned()].as_slice())
        );
        let empty: &[String] = &[];
        let second = argv.get(1).map_or(empty, Vec::as_slice);
        assert_eq!(second.first().map(String::as_str), Some("tmux"));
        assert!(second.windows(2).any(|pair| pair == ["-L", "other"]));
    }
    let mut recorded = Observation {
        stdout: String::new(),
        stderr: String::new(),
        exit: 0,
        json: None,
        files: std::collections::BTreeMap::new(),
        tmux_argv: vec![vec![
            "tmux".into(),
            "-b".into(),
            "scratch".into(),
            "/tmp/session.sock".into(),
        ]],
    };
    recorded =
        normalize_observation(&recorded, &[NormRule::SocketPath, NormRule::AbsPath], &[]).unwrap();
    let flat = recorded.tmux_argv.first().unwrap().join(" ");
    assert!(flat.contains("<BUF:1>"), "{flat}");
    assert!(flat.contains("<SOCK:1>"), "{flat}");
    assert!(!flat.contains("scratch"));
}

#[test]
fn missing_typescript_tree_exits_2_and_does_not_skip() {
    let home = tempfile::TempDir::with_prefix("rigrs-home-").unwrap();
    let (code, _stdout, stderr) = run(
        &["diff", "--bin", "/tmp/rig-does-not-need-to-exist"],
        Some(home.path()),
        &["OPENRIG_DIR"],
    )
    .unwrap();
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("will not skip"), "{stderr}");
    assert!(stderr.contains("v0.6.9"), "{stderr}");
    assert!(stderr.contains("4b48ca21"), "{stderr}");
    assert!(stderr.contains("Node >= 22"), "{stderr}");
}

#[test]
fn self_mode_exits_0_and_update_with_self_is_a_usage_error() {
    let dir = tempfile::TempDir::with_prefix("rigrs-bin-").unwrap();
    let bin = dir.path().join("rig");
    fs::write(&bin, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
    let bin_text = bin.display().to_string();
    let (code, stdout, stderr) = run(&["diff", "--self", "--bin", &bin_text], None, &[]).unwrap();
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    assert!(stdout.contains("| version | Equal |"), "{stdout}");
    assert!(stdout.contains("needs UDS listener"), "{stdout}");
    assert!(stdout.contains("needs slice 1"), "{stdout}");
    let (code, _stdout, stderr) = run(
        &["diff", "--self", "--update", "--bin", &bin_text],
        None,
        &[],
    )
    .unwrap();
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("not valid with --self"), "{stderr}");
}

#[test]
fn ledger_check_and_report_exit_0() {
    let (code, stdout, stderr) = run(&["ledger", "check"], None, &[]).unwrap();
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("ledger ok"), "{stdout}");
    let (code, stdout, stderr) = run(&["ledger", "report"], None, &[]).unwrap();
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("| todo |"), "{stdout}");
    assert!(stdout.contains("| waived |"), "{stdout}");
}
