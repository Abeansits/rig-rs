//! `rig setup` plan, apply, and install confirmation.
#![expect(
    clippy::unwrap_used,
    reason = "setup tests assert temp-file results directly"
)]

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

use assert_cmd::Command;

fn rig(home: &Path, rig_home: &Path) -> Command {
    let mut cmd = Command::cargo_bin("rig").unwrap();
    cmd.env("HOME", home)
        .env("RIG_HOME", rig_home)
        .env_remove("RIG_DOCTOR_FIXTURE")
        .env_remove("CODEX_HOME");
    cmd
}

#[test]
fn dry_run_json_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let rig_home = dir.path().join("rig-home");
    fs::create_dir(&home).unwrap();
    let sentinel = home.join(".tmux.conf");
    fs::write(&sentinel, "keep\n").unwrap();
    let before = fs::metadata(&sentinel).unwrap();
    let output = rig(&home, &rig_home)
        .args(["setup", "--dry-run", "--json", "--no-herdr", "--no-ghostty"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(output).unwrap();
    let plan: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(
        plan["steps"]
            .as_array()
            .is_some_and(|steps| steps.len() >= 4)
    );
    let after = fs::metadata(&sentinel).unwrap();
    assert_eq!(after.ino(), before.ino());
    assert_eq!(after.modified().unwrap(), before.modified().unwrap());
    assert_eq!(fs::read(&sentinel).unwrap(), b"keep\n");
    assert!(!rig_home.exists());
}

#[test]
fn apply_is_idempotent_and_skips_unselected_providers() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let rig_home = dir.path().join("rig-home");
    fs::create_dir(&home).unwrap();
    rig(&home, &rig_home)
        .args([
            "setup",
            "--providers",
            "claude",
            "--no-herdr",
            "--no-ghostty",
        ])
        .assert()
        .success();
    let seat = rig_home.join("seats").join("default");
    assert!(seat.join("mcp.json").is_file());
    assert!(seat.join("settings.json").is_file());
    assert!(!seat.join("opencode.json").exists());
    assert!(!home.join(".codex").exists());
    assert!(
        rig_home
            .join("skills")
            .join("using-rig")
            .join("SKILL.md")
            .is_file()
    );
    assert!(home.join(".tmux.conf").is_file());
    let user_config = home.join(crewmux_provider::user_config_basename());
    assert!(!user_config.exists());
    let mcp = fs::read(seat.join("mcp.json")).unwrap();
    let tmux = fs::read(home.join(".tmux.conf")).unwrap();
    rig(&home, &rig_home)
        .args([
            "setup",
            "--providers",
            "claude",
            "--no-herdr",
            "--no-ghostty",
        ])
        .assert()
        .success();
    assert_eq!(fs::read(seat.join("mcp.json")).unwrap(), mcp);
    assert_eq!(fs::read(home.join(".tmux.conf")).unwrap(), tmux);
}

#[test]
fn json_without_yes_does_not_install() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let rig_home = dir.path().join("rig-home");
    let bin = dir.path().join("bin");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&bin).unwrap();
    let marker = dir.path().join("installed");
    let script = format!("#!/bin/sh\ntouch {}\n", marker.display());
    let curl = bin.join("curl");
    fs::write(&curl, script).unwrap();
    let mut mode = fs::metadata(&curl).unwrap().permissions();
    mode.set_mode(0o755);
    fs::set_permissions(&curl, mode).unwrap();
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    rig(&home, &rig_home)
        .env("PATH", &path)
        .args(["setup", "--json", "--no-ghostty"])
        .assert()
        .success();
    assert!(!marker.exists());
    rig(&home, &rig_home)
        .env("PATH", &path)
        .args(["setup", "--yes", "--json", "--no-herdr", "--no-ghostty"])
        .assert()
        .success();
    assert!(!marker.exists());
    rig(&home, &rig_home)
        .env("PATH", &path)
        .args(["setup", "--yes", "--no-ghostty"])
        .assert()
        .success();
    assert!(
        !marker.exists(),
        "--yes alone must not run the herdr installer"
    );
}

#[test]
fn setup_does_not_trust_the_home_directory() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let rig_home = dir.path().join("rig-home");
    fs::create_dir(&home).unwrap();
    rig(&home, &rig_home)
        .args([
            "setup",
            "--providers",
            "codex",
            "--no-herdr",
            "--no-ghostty",
        ])
        .assert()
        .success();
    let config = fs::read_to_string(home.join(".codex").join("config.toml")).unwrap();
    assert!(config.contains("[mcp_servers.rig]"));
    assert!(!config.contains(&home.display().to_string()));
    assert!(!config.contains("trust_level"));
}

#[test]
fn dry_run_after_apply_reports_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let rig_home = dir.path().join("rig-home");
    fs::create_dir(&home).unwrap();
    rig(&home, &rig_home)
        .args(["setup", "--no-herdr", "--no-ghostty"])
        .assert()
        .success();
    let output = rig(&home, &rig_home)
        .args(["setup", "--dry-run", "--json", "--no-herdr", "--no-ghostty"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let plan: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let steps = plan["steps"].as_array().unwrap();
    for id in [
        "claude_mcp",
        "claude_settings",
        "codex",
        "opencode",
        "tmux_conf",
    ] {
        let step = steps
            .iter()
            .find(|step| step["id"] == id)
            .unwrap_or_else(|| panic!("missing {id}"));
        assert_eq!(step["change"], "unchanged", "{id} {step}");
    }
}

#[test]
fn read_only_tmux_conf_is_left_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let rig_home = dir.path().join("rig-home");
    fs::create_dir(&home).unwrap();
    let conf = home.join(".tmux.conf");
    fs::write(&conf, "keep\n").unwrap();
    let mut permissions = fs::metadata(&conf).unwrap().permissions();
    permissions.set_mode(0o444);
    fs::set_permissions(&conf, permissions).unwrap();
    rig(&home, &rig_home)
        .args([
            "setup",
            "--providers",
            "claude",
            "--no-herdr",
            "--no-ghostty",
        ])
        .assert()
        .failure();
    assert_eq!(fs::read(&conf).unwrap(), b"keep\n");
}
