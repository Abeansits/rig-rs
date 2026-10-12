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
                signal_if_owned(&rig_home, pid, "-TERM");
            }
            thread::park_timeout(Duration::from_millis(50));
        }
        for pid in daemon_run_pids(&rig_home) {
            signal_if_owned(&rig_home, pid, "-KILL");
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
    // macOS has no `/proc`, and SIP hides another process's environment, so
    // the pid scan below cannot see this child. The file is what `finish`
    // kills, and only after the start time still matches.
    remember_pid(&rig_home, child.id())?;
    wait_health(&socket, &mut child, &err_path)
}

fn remember_pid(rig_home: &Path, pid: u32) -> Result<(), Error> {
    let stamp = process_stamp(pid).unwrap_or_else(|| "-".to_owned());
    let path = rig_home.join("run/harness.pid");
    std::fs::write(path, format!("{pid} {stamp}"))
        .map_err(|source| io_err("record daemon pid", source))
}

/// `{pid} {stamp}` from `run/harness.pid`. The stamp may contain spaces.
fn harness_record(rig_home: &Path) -> Option<(u32, String)> {
    let text = std::fs::read_to_string(rig_home.join("run/harness.pid")).ok()?;
    split_record(text.trim())
}

fn split_record(text: &str) -> Option<(u32, String)> {
    let (pid, stamp) = text.split_once(char::is_whitespace)?;
    let pid = pid.parse().ok()?;
    let stamp = stamp.trim();
    if stamp.is_empty() {
        None
    } else {
        Some((pid, stamp.to_owned()))
    }
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

/// Signal only while this pid is still the daemon recorded for `rig_home`.
fn signal_if_owned(rig_home: &Path, pid: u32, name: &str) {
    if still_owned(rig_home, pid) {
        signal(pid, name);
    }
}

/// `daemon run` plus this home. Environ is exact `RIG_HOME` when readable.
/// Otherwise the start stamp in `run/harness.pid` must still match, so a
/// recycled pid is never killed. `-` is not a stamp.
fn still_owned(rig_home: &Path, pid: u32) -> bool {
    if !command_line(pid).is_some_and(|cmd| is_daemon_run(&cmd)) {
        return false;
    }
    let home = rig_home.display().to_string();
    match read_environ(pid) {
        Some(env) => env_eq(&env, "RIG_HOME", &home),
        None => harness_record(rig_home).is_some_and(|(recorded, stamp)| {
            recorded == pid && process_stamp(pid).is_some_and(|now| stamps_agree(&stamp, &now))
        }),
    }
}

fn stamps_agree(stored: &str, current: &str) -> bool {
    !stored.is_empty() && stored != "-" && stored == current
}

fn read_environ(pid: u32) -> Option<Vec<u8>> {
    let path = PathBuf::from(format!("/proc/{pid}/environ"));
    if !path.is_file() {
        return None;
    }
    std::fs::read(path).ok()
}

/// Absolute start identity. Linux uses `/proc` start ticks. Other platforms
/// use `ps -o lstart=`, which stays equal for the life of the process.
fn process_stamp(pid: u32) -> Option<String> {
    if let Some(ticks) = proc_start_ticks(pid) {
        return Some(ticks.to_string());
    }
    ps_column(pid, "lstart=")
}

fn proc_start_ticks(pid: u32) -> Option<u64> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = text.rsplit_once(')')?.1;
    rest.split_whitespace().nth(19)?.parse().ok()
}

fn ps_column(pid: u32, column: &str) -> Option<String> {
    let pid_text = pid.to_string();
    let mut command = Command::new("ps");
    command
        .args(["-ww", "-p", &pid_text, "-o", column])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let child = command.spawn().ok()?;
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_owned())
    }
}

/// Process ids of `rig daemon run` for this `RIG_HOME`.
///
/// Linux matches `/proc/<pid>/environ`. That directory is absent on macOS, so
/// a daemon this harness spawned is also taken from `run/harness.pid` when
/// the command is still `daemon run` and the start stamp still matches.
#[must_use]
pub(crate) fn daemon_run_pids(rig_home: &Path) -> Vec<u32> {
    let mut pids = pids_from_proc(rig_home);
    if let Some((pid, _)) = harness_record(rig_home)
        && !pids.contains(&pid)
        && still_owned(rig_home, pid)
    {
        pids.push(pid);
    }
    pids
}

fn pids_from_proc(rig_home: &Path) -> Vec<u32> {
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

fn command_line(pid: u32) -> Option<Vec<u8>> {
    let dir = PathBuf::from(format!("/proc/{pid}"));
    if dir.is_dir() {
        let bytes = std::fs::read(dir.join("cmdline")).unwrap_or_default();
        if bytes.is_empty() { None } else { Some(bytes) }
    } else {
        ps_command_line(pid)
    }
}

fn ps_command_line(pid: u32) -> Option<Vec<u8>> {
    let pid_text = pid.to_string();
    let mut command = Command::new("ps");
    command
        .args(["-ww", "-p", &pid_text, "-o", "args="])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let child = command.spawn().ok()?;
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    cmdline_from_ps_text(&text)
}

/// `ps -o args=` is space-separated. A zombie is `<defunct>` and is not a daemon.
fn cmdline_from_ps_text(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if text.is_empty() || text.contains("<defunct>") {
        return None;
    }
    let mut bytes = Vec::new();
    for arg in text.split_whitespace() {
        bytes.extend_from_slice(arg.as_bytes());
        bytes.push(0);
    }
    if bytes.is_empty() { None } else { Some(bytes) }
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
    use super::{
        cmdline_from_ps_text, env_eq, is_daemon_run, process_stamp, split_record, stamps_agree,
    };

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

    #[test]
    fn ps_text_matches_a_shebang_daemon_and_skips_a_zombie() {
        let cmd = cmdline_from_ps_text("python3 /var/folders/xx/rig daemon run\n");
        assert!(cmd.as_deref().is_some_and(is_daemon_run));
        let env_shebang =
            cmdline_from_ps_text("/usr/bin/env python3 /tmp/rigrs-bin/rig daemon run");
        assert!(env_shebang.as_deref().is_some_and(is_daemon_run));
        assert!(cmdline_from_ps_text("  <defunct>  ").is_none());
        assert!(cmdline_from_ps_text("").is_none());
        let status = cmdline_from_ps_text("python3 /tmp/rig daemon status");
        assert!(status.is_some_and(|cmd| !is_daemon_run(&cmd)));
    }

    #[test]
    fn a_stamp_must_match_and_a_dash_never_authorizes_a_kill() {
        assert_eq!(
            split_record("4242 Mon Oct 12 06:27:32 2026"),
            Some((4242, "Mon Oct 12 06:27:32 2026".to_owned()))
        );
        assert_eq!(split_record("7 99"), Some((7, "99".to_owned())));
        assert!(split_record("4242").is_none());
        assert!(split_record("nope stamp").is_none());
        assert!(split_record("1 ").is_none());
        assert!(stamps_agree("99", "99"));
        assert!(stamps_agree(
            "Mon Oct 12 06:27:32 2026",
            "Mon Oct 12 06:27:32 2026"
        ));
        assert!(!stamps_agree("-", "-"));
        assert!(!stamps_agree("", "99"));
        assert!(!stamps_agree("99", "100"));
    }

    #[test]
    fn this_process_stamp_is_stable_when_readable() {
        let Some(stamp) = process_stamp(std::process::id()) else {
            return;
        };
        assert_ne!(stamp, "-");
        assert_eq!(
            process_stamp(std::process::id()).as_deref(),
            Some(stamp.as_str())
        );
    }
}
