//! AC8. Runs `codex mcp list` when a `codex` binary is on `PATH`.

use std::fs;
use std::process::{Command, Stdio};

use crewmux_provider::{CodexConfig, RIG_MCP_NAME, rig_mcp_command};

#[test]
#[cfg_attr(
    not(codex_binary_present),
    ignore = "skipped: codex binary is not present"
)]
fn codex_mcp_list_lists_our_server() {
    let Some(codex) = which("codex") else {
        eprintln!("skipped: codex binary is not present");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let codex_home = dir.path().join("codex-home");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&codex_home).unwrap();
    let config_path = codex_home.join("config.toml");
    let mut config = CodexConfig::load(&config_path).unwrap();
    config
        .ensure_mcp_server(RIG_MCP_NAME, &rig_mcp_command())
        .unwrap();
    let backups = crewmux_provider::Backups::new(&dir.path().join("rig-home"));
    config.save(&backups).unwrap();
    let mut command = Command::new(&codex);
    command
        .arg("mcp")
        .arg("list")
        .env("HOME", &home)
        .env("CODEX_HOME", &codex_home)
        .env_remove("RIG_DOCTOR_FIXTURE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command.spawn().unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "codex mcp list failed: {stderr}");
    assert!(
        stdout.contains(RIG_MCP_NAME),
        "stdout={stdout} stderr={stderr}"
    );
}

fn which(name: &str) -> Option<std::path::PathBuf> {
    for dir in std::env::var("PATH").unwrap_or_default().split(':') {
        let candidate = std::path::PathBuf::from(dir).join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}
