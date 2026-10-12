//! Temp homes for CLI tests. The real `$HOME` is never the process home.
#![expect(
    clippy::expect_used,
    reason = "setup failures should abort the integration test"
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use assert_cmd::cargo::CommandCargoExt;

pub(crate) struct Env {
    pub(crate) root: tempfile::TempDir,
    pub(crate) home: PathBuf,
    pub(crate) rig_home: PathBuf,
}

impl Env {
    pub(crate) fn new() -> Self {
        let root = tempfile::TempDir::with_prefix("rig-slice1-").expect("temp");
        let home = root.path().join("home");
        let rig_home = root.path().join("rig-home");
        std::fs::create_dir_all(&home).expect("home");
        std::fs::create_dir_all(&rig_home).expect("rig");
        Self {
            root,
            home,
            rig_home,
        }
    }

    pub(crate) fn rig(&self) -> Command {
        let mut command = Command::cargo_bin("rig").expect("rig");
        command
            .env("HOME", &self.home)
            .env("RIG_HOME", &self.rig_home)
            .env("RIG_FAKE_MUX", "1")
            .env_remove("RIG_STUB_SCRIPT")
            .env_remove("RIG_TMUX_SOCKET")
            .env_remove("TMUX")
            .env_remove("TMUX_TMPDIR")
            .current_dir(&self.home);
        command
    }

    pub(crate) fn write_spec(&self, name: &str, body: &str) -> PathBuf {
        let path = self.home.join(name);
        std::fs::write(&path, body).expect("spec");
        path
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let socket = crewmux_api::daemon_socket(&self.rig_home);
        let client = crewmux_api::Client::over_uds(socket).with_timeout(Duration::from_secs(2));
        drop(client.request("POST", "/shutdown", Some(b"{}")));
    }
}

pub(crate) fn two_stub(cwd: &Path) -> String {
    let cwd = cwd.display();
    format!(
        "name: pair\nseats:\n  - name: one\n    runtime: stub\n    cwd: {cwd}\n  - name: two\n    runtime: stub\n    cwd: {cwd}\n"
    )
}

pub(crate) fn stdout(command: &mut Command) -> (i32, String, String) {
    let output = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rig")
        .wait_with_output()
        .expect("wait rig");
    let code = output.status.code().unwrap_or(1);
    let out = String::from_utf8_lossy(&output.stdout).into_owned();
    let err = String::from_utf8_lossy(&output.stderr).into_owned();
    (code, out, err)
}
