//! `rig doctor --json` goldens. The fixture keeps the report off the host PATH.
#![expect(
    clippy::unwrap_used,
    reason = "golden tests assert command output directly"
)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use assert_cmd::Command;
use crewmux_provider::render_tmux_block;

fn fixture(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    path
}

fn base(conf: &str) -> String {
    format!(
        r#"{{
  "os": "macos",
  "tmux_version": "tmux 3.5a",
  "tmux_conf": {conf},
  "window_size": "latest",
  "daemon_up": true,
  "claude": {{"present": true, "version": "2.1.0", "logged_in": true}},
  "claude_running": false,
  "codex": {{"present": true, "version": "0.40.0", "logged_in": true}},
  "codex_config": "current",
  "opencode": {{"present": true, "version": "1.19.0", "logged_in": false}},
  "herdr": "22",
  "ghostty": {{"present": true, "version": "1.3.0", "logged_in": false}},
  "ghostty_aqua": true,
  "rig_home": "ok"
}}"#
    )
}

fn run(home: &Path, fixture_path: &Path, extra: &[&str]) -> (i32, String) {
    let mut cmd = Command::cargo_bin("rig").unwrap();
    cmd.env("HOME", home)
        .env("RIG_HOME", home.join("rig-home"))
        .env("RIG_DOCTOR_FIXTURE", fixture_path)
        .args(["doctor", "--json"])
        .args(extra);
    let asserted = cmd.assert();
    let output = asserted.get_output();
    let code = output.status.code().unwrap_or(1);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    (code, stdout)
}

#[test]
fn json_scenarios_match_goldens() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    fs::create_dir(&home).unwrap();
    let conf = serde_json::to_string(&render_tmux_block()).unwrap();
    let cases = [
        ("all_pass", base(&conf), &[][..], 0),
        (
            "tmux_3_2",
            base(&conf).replace("tmux 3.5a", "tmux 3.2"),
            &[][..],
            1,
        ),
        (
            "window_size_manual",
            base(&conf).replace("\"latest\"", "\"manual\""),
            &[][..],
            1,
        ),
        (
            "codex_not_logged_in",
            base(&conf).replace(
                "\"version\": \"0.40.0\", \"logged_in\": true",
                "\"version\": \"0.40.0\", \"logged_in\": false",
            ),
            &[][..],
            0,
        ),
        (
            "opencode_mismatch",
            base(&conf).replace("1.19.0", "1.18.0"),
            &[][..],
            0,
        ),
        (
            "herdr_protocol_mismatch",
            base(&conf).replace("\"herdr\": \"22\"", "\"herdr\": \"21\""),
            &[][..],
            0,
        ),
        (
            "ghostty_skipped",
            base(&conf).replace("\"os\": \"macos\"", "\"os\": \"linux\""),
            &[][..],
            0,
        ),
    ];
    for (name, body, extra, code) in cases {
        let path = fixture(dir.path(), &format!("{name}.json"), &body);
        let (status, stdout) = run(&home, &path, extra);
        assert_eq!(status, code, "{name}\n{stdout}");
        let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(parsed["healthy"], code == 0, "{name}");
        for check in parsed["checks"].as_array().unwrap() {
            let status = check["status"].as_str().unwrap();
            if status == "fail" || status == "warn" {
                assert!(check.get("fix").is_some(), "{name} {check}");
            }
        }
        insta::assert_snapshot!(name, stdout);
    }
}

#[test]
fn no_server_manual_conf_fails_window_size() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let bin = dir.path().join("bin");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&bin).unwrap();
    let tmux = bin.join("tmux");
    fs::write(
        &tmux,
        "#!/bin/sh\nif [ \"$1\" = \"-V\" ]; then echo \"tmux 3.5a\"; exit 0; fi\necho \"no server running\" >&2\nexit 1\n",
    )
    .unwrap();
    let mut mode = fs::metadata(&tmux).unwrap().permissions();
    mode.set_mode(0o755);
    fs::set_permissions(&tmux, mode).unwrap();
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    fs::write(home.join(".tmux.conf"), "set -g window-size manual\n").unwrap();
    assert_eq!(
        window_size_status(&home, &path),
        "fail",
        "no server and a manual conf must not pass"
    );
    fs::write(home.join(".tmux.conf"), "set -g mouse on\n").unwrap();
    assert_eq!(window_size_status(&home, &path), "pass");
}

fn window_size_status(home: &Path, path: &str) -> String {
    let mut cmd = Command::cargo_bin("rig").unwrap();
    let asserted = cmd
        .env("HOME", home)
        .env("RIG_HOME", home.join("rig-home"))
        .env("PATH", path)
        .env_remove("RIG_DOCTOR_FIXTURE")
        .env_remove("TMUX")
        .args(["doctor", "--json", "--no-herdr"])
        .assert();
    let stdout = String::from_utf8_lossy(&asserted.get_output().stdout).into_owned();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    parsed
        .get("checks")
        .and_then(serde_json::Value::as_array)
        .and_then(|checks| {
            checks.iter().find(|check| {
                check.get("name").and_then(serde_json::Value::as_str)
                    == Some("tmux_global_window_size")
            })
        })
        .and_then(|check| check.get("status"))
        .and_then(serde_json::Value::as_str)
        .unwrap()
        .to_owned()
}

#[test]
fn doctor_does_not_create_rig_home_or_start_a_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let rig_home = dir.path().join("missing-rig-home");
    fs::create_dir(&home).unwrap();
    let mut cmd = Command::cargo_bin("rig").unwrap();
    cmd.env("HOME", &home)
        .env("RIG_HOME", &rig_home)
        .env_remove("RIG_DOCTOR_FIXTURE")
        .args(["doctor", "--json", "--no-herdr"])
        .assert()
        .code(1);
    assert!(!rig_home.exists(), "doctor created RIG_HOME");
}
