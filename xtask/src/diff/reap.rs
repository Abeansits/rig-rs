//! Stop daemons a diff case started, and fail if any `rig daemon run` remains.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use super::types::{HermeticHome, Side};
use crate::error::{Error, io_err};

/// Owns the scratch home's daemons until [`DaemonGuard::finish`].
pub(crate) struct DaemonGuard<'a> {
    home: &'a HermeticHome,
    side: &'a Side,
    port: Option<u16>,
}

impl<'a> DaemonGuard<'a> {
    pub(crate) fn new(home: &'a HermeticHome, side: &'a Side, port: Option<u16>) -> Self {
        Self { home, side, port }
    }

    /// `GET /healthz` target is up. Starts the product daemon when it is not.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the daemon cannot be spawned or never answers.
    pub(crate) fn ensure(&self) -> Result<(), Error> {
        match self.side {
            Side::Rs { bin } => ensure_rs(bin, self.home),
            Side::Ts { node, pkg } => {
                let port = self.port.unwrap_or(9);
                ensure_ts(node, pkg, port, self.home)
            }
        }
    }

    /// Shut the daemon down and assert no `rig daemon run` for this home remains.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when a daemon process is still alive after SIGKILL.
    pub(crate) fn finish(&self) -> Result<(), Error> {
        let rig_home = self.home.rig_home();
        let socket = crewmux_api::daemon_socket(&rig_home);
        drop(
            crewmux_api::Client::over_uds(&socket)
                .with_timeout(Duration::from_secs(2))
                .request("POST", "/shutdown", Some(b"{}")),
        );
        if let Side::Ts { node, pkg } = self.side {
            drop(stop_ts(node, pkg, self.home));
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && !daemon_run_pids(&rig_home).is_empty() {
            for pid in daemon_run_pids(&rig_home) {
                signal(pid, "-TERM");
            }
            thread::park_timeout(Duration::from_millis(50));
        }
        for pid in daemon_run_pids(&rig_home) {
            signal(pid, "-KILL");
        }
        thread::park_timeout(Duration::from_millis(50));
        let left = daemon_run_pids(&rig_home);
        if left.is_empty() {
            Ok(())
        } else {
            Err(Error::Message(format!(
                "rig daemon run still alive for {}: {left:?}",
                rig_home.display()
            )))
        }
    }
}

fn ensure_rs(bin: &Path, home: &HermeticHome) -> Result<(), Error> {
    let rig_home = home.rig_home();
    std::fs::create_dir_all(rig_home.join("run"))
        .map_err(|source| io_err("create daemon run dir", source))?;
    let socket = crewmux_api::daemon_socket(&rig_home);
    if health_ok(&socket) {
        return Ok(());
    }
    let err_path = rig_home.join("run/daemon.err");
    let err_file = std::fs::File::create(&err_path)
        .map_err(|source| io_err("create daemon stderr", source))?;
    let mut command = Command::new(bin);
    command
        .args(["daemon", "run"])
        .env_clear()
        .envs(home.child_env())
        .current_dir(&home.home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(err_file));
    command.process_group(0);
    let mut child = command
        .spawn()
        .map_err(|source| io_err("spawn rig daemon run", source))?;
    wait_health(&socket, &mut child, &err_path)
}

fn ensure_ts(node: &Path, pkg: &Path, port: u16, home: &HermeticHome) -> Result<(), Error> {
    let wrapper = pkg.join("packages/cli/dist/bin-wrapper.js");
    let mut command = Command::new(node);
    command
        .arg(&wrapper)
        .args([
            "daemon",
            "start",
            "--no-kernel",
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
        ])
        .env_clear()
        .envs(home.child_env())
        .current_dir(&home.home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command
        .spawn()
        .map_err(|source| io_err("spawn openrig daemon start", source))?;
    let output = child
        .wait_with_output()
        .map_err(|source| io_err("wait openrig daemon start", source))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Error::Message(format!(
            "openrig daemon start failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

fn stop_ts(node: &Path, pkg: &Path, home: &HermeticHome) -> Result<(), Error> {
    let wrapper = pkg.join("packages/cli/dist/bin-wrapper.js");
    let mut command = Command::new(node);
    command
        .arg(wrapper)
        .args(["daemon", "stop"])
        .env_clear()
        .envs(home.child_env())
        .current_dir(&home.home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
        .status()
        .map_err(|source| io_err("openrig daemon stop", source))?;
    Ok(())
}

fn health_ok(socket: &Path) -> bool {
    crewmux_api::Client::over_uds(socket)
        .with_timeout(Duration::from_millis(200))
        .request("GET", "/healthz", None)
        .is_ok_and(|response| response.status == 200)
}

fn wait_health(
    socket: &Path,
    child: &mut std::process::Child,
    err_path: &Path,
) -> Result<(), Error> {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if health_ok(socket) {
            return Ok(());
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|source| io_err("wait for rig daemon run", source))?
        {
            return Err(Error::Message(format!(
                "rig daemon run exited {} before /healthz: {}",
                status.code().unwrap_or(1),
                stderr_tail(err_path)
            )));
        }
        thread::park_timeout(Duration::from_millis(50));
    }
    Err(Error::Message(format!(
        "daemon did not answer /healthz: {}",
        stderr_tail(err_path)
    )))
}

fn stderr_tail(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let text = text.trim();
    if text.len() <= 500 {
        return text.to_owned();
    }
    text.get(text.len() - 500..).unwrap_or(text).to_owned()
}

fn signal(pid: u32, name: &str) {
    drop(
        Command::new("kill")
            .args([name, &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status(),
    );
}

/// Process ids of `rig daemon run` whose environment names this `RIG_HOME`.
#[must_use]
pub(crate) fn daemon_run_pids(rig_home: &Path) -> Vec<u32> {
    let home = rig_home.display().to_string();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut pids = Vec::new();
    for entry in entries.flatten() {
        let Some(text) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(pid) = text.parse::<u32>() else {
            continue;
        };
        if pid == std::process::id() {
            continue;
        }
        let dir: PathBuf = entry.path();
        let cmdline = std::fs::read(dir.join("cmdline")).unwrap_or_default();
        if !is_daemon_run(&cmdline) {
            continue;
        }
        let env = std::fs::read(dir.join("environ")).unwrap_or_default();
        if env_eq(&env, "RIG_HOME", &home) {
            pids.push(pid);
        }
    }
    pids
}

fn is_daemon_run(cmdline: &[u8]) -> bool {
    let args: Vec<&[u8]> = cmdline
        .split(|byte| *byte == 0)
        .filter(|arg| !arg.is_empty())
        .collect();
    args.iter().enumerate().any(|(index, arg)| {
        if *arg != b"daemon" {
            return false;
        }
        if args.get(index.saturating_add(1)).copied() != Some(b"run".as_slice()) {
            return false;
        }
        // A real binary is `rig` or `…/rig`. A `#!/usr/bin/env python3` script
        // at that path shows up as `python3`, then the script, then `daemon`.
        let Some(program) = index.checked_sub(1).and_then(|prev| args.get(prev)) else {
            return false;
        };
        *program == b"rig" || program.ends_with(b"/rig")
    })
}

fn env_eq(env: &[u8], key: &str, value: &str) -> bool {
    let needle = format!("{key}={value}");
    env.split(|byte| *byte == 0)
        .any(|entry| entry == needle.as_bytes())
}

/// Bind an ephemeral loopback port and release it.
///
/// # Errors
///
/// Returns [`Error`] when the socket cannot be bound.
pub(crate) fn free_port() -> Result<u16, Error> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|source| io_err("bind ephemeral port", source))?;
    let port = listener
        .local_addr()
        .map_err(|source| io_err("read ephemeral port", source))?
        .port();
    drop(listener);
    Ok(port)
}

#[cfg(test)]
mod tests {
    use super::{env_eq, is_daemon_run};

    #[test]
    fn daemon_match_is_the_exact_home_and_argv() {
        let cmd = b"/usr/bin/rig\0daemon\0run\0";
        assert!(is_daemon_run(cmd));
        assert!(is_daemon_run(b"python3\0/tmp/rigrs-bin/rig\0daemon\0run\0"));
        assert!(is_daemon_run(
            b"/usr/bin/env\0python3\0/tmp/rig\0daemon\0run\0"
        ));
        assert!(!is_daemon_run(b"/usr/bin/rig\0daemon\0status\0"));
        assert!(!is_daemon_run(b"/usr/bin/not-rig\0daemon\0run\0"));
        assert!(!is_daemon_run(b"python3\0/tmp/not-rig\0daemon\0run\0"));
        let env = b"PATH=/bin\0RIG_HOME=/tmp/scratch\0HOME=/tmp/scratch\0";
        assert!(env_eq(env, "RIG_HOME", "/tmp/scratch"));
        assert!(!env_eq(env, "RIG_HOME", "/tmp"));
        assert!(!env_eq(env, "RIG_HOME", "/tmp/scratch-other"));
        assert!(!env_eq(env, "HOME", "/tmp/scratch/nested"));
    }
}
