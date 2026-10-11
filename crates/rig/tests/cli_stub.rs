//! `rig up|ps|capture|attach|down` against a real daemon and `FakeMux`.
#![expect(
    clippy::expect_used,
    reason = "setup failures should abort the integration test"
)]

mod common;

use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use assert_cmd::cargo::CommandCargoExt;

use common::{Env, stdout, two_stub};
use serde_json::Value;

fn json_out(command: &mut std::process::Command) -> (i32, Value, String) {
    let (code, out, err) = stdout(command);
    let value = serde_json::from_str(out.trim()).unwrap_or(Value::Null);
    (code, value, err)
}

#[test]
fn up_ps_capture_down_json() {
    let env = Env::new();
    let spec = env.write_spec("two-stub.yaml", &two_stub(&env.home));
    let (code, up, err) = json_out(
        env.rig()
            .args(["up", spec.to_str().expect("utf8"), "--json"]),
    );
    assert_eq!(code, 0, "{err} {up}");
    assert_eq!(up.get("status").and_then(Value::as_str), Some("ready"));
    insta::assert_snapshot!("up", serde_json::to_string_pretty(&up).expect("json"));
    let (code, listed, err) = json_out(env.rig().args(["ps", "--json"]));
    assert_eq!(code, 0, "{err}");
    insta::assert_snapshot!("ps", serde_json::to_string_pretty(&listed).expect("json"));
    let (code, shot, err) = json_out(env.rig().args(["capture", "one", "--json"]));
    assert_eq!(code, 0, "{err}");
    let text = shot.get("text").and_then(Value::as_str).unwrap_or_default();
    assert!(text.contains("stub> "), "{shot}");
    let (code, down, err) = json_out(env.rig().args(["down", "--json"]));
    assert_eq!(code, 0, "{err}");
    assert_eq!(down.get("sessions_killed").and_then(Value::as_u64), Some(2));
    insta::assert_snapshot!("down", serde_json::to_string_pretty(&down).expect("json"));
}

#[test]
fn second_up_is_already_running() {
    let env = Env::new();
    let spec = env.write_spec("two-stub.yaml", &two_stub(&env.home));
    let path = spec.to_str().expect("utf8");
    let (code, _, err) = json_out(env.rig().args(["up", path, "--json"]));
    assert_eq!(code, 0, "{err}");
    let (code, again, err) = json_out(env.rig().args(["up", path, "--json"]));
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        again.get("status").and_then(Value::as_str),
        Some("already_running")
    );
}

#[test]
fn capture_lines_are_bounded() {
    let env = Env::new();
    let spec = env.write_spec("two-stub.yaml", &two_stub(&env.home));
    let (code, _, err) = json_out(
        env.rig()
            .args(["up", spec.to_str().expect("utf8"), "--json"]),
    );
    assert_eq!(code, 0, "{err}");
    let (code, shot, err) = json_out(env.rig().args(["capture", "one", "--lines", "1", "--json"]));
    assert_eq!(code, 0, "{err}");
    assert_eq!(shot.get("lines").and_then(Value::as_u64), Some(1));
    let text = shot.get("text").and_then(Value::as_str).unwrap_or_default();
    assert!(text.contains("stub> "), "{text}");
    let (code, plain, err) = stdout(env.rig().args(["capture", "one"]));
    assert_eq!(code, 0, "{err}");
    assert!(
        plain.contains("stub> "),
        "plain capture should print pane content: {plain}"
    );
}

/// A recycled pid in `daemon.pid` is not the daemon. The lock and the socket
/// decide liveness, so `up` still starts.
#[test]
fn stale_pid_does_not_block_the_daemon() {
    let env = Env::new();
    let run = env.rig_home.join("run");
    std::fs::create_dir_all(&run).expect("run");
    let mut sleep = std::process::Command::new("sleep")
        .arg("30")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("sleep");
    std::fs::write(run.join("daemon.pid"), sleep.id().to_string()).expect("pid");
    let spec = env.write_spec("two-stub.yaml", &two_stub(&env.home));
    let (code, up, err) = json_out(
        env.rig()
            .args(["up", spec.to_str().expect("utf8"), "--json"]),
    );
    drop(sleep.kill());
    drop(sleep.wait());
    assert_eq!(code, 0, "{err} {up}");
    assert_eq!(up.get("status").and_then(Value::as_str), Some("ready"));
}

/// PE-008: `rig attach` execs `tmux -L rig-rs attach -t =<session>`.
#[test]
fn attach_execs_exact_tmux_target() {
    let env = Env::new();
    let spec = env.write_spec("two-stub.yaml", &two_stub(&env.home));
    let (code, _, err) = json_out(
        env.rig()
            .args(["up", spec.to_str().expect("utf8"), "--json"]),
    );
    assert_eq!(code, 0, "{err}");
    let bin = env.root.path().join("bin");
    std::fs::create_dir_all(&bin).expect("bin");
    let script = bin.join("tmux");
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$TMUX_ARGV_OUT\"\nexit 0\n",
    )
    .expect("script");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("mode");
    let log = env.root.path().join("tmux-argv");
    let path = std::env::var("PATH").unwrap_or_default();
    let (code, _, err) = stdout(
        env.rig()
            .args(["attach", "one"])
            .env("TMUX_ARGV_OUT", &log)
            .env("PATH", format!("{}:{path}", bin.display()))
            .env_remove("RIG_TMUX_SOCKET"),
    );
    assert_eq!(code, 0, "{err}");
    let text = std::fs::read_to_string(&log).expect("argv");
    assert_eq!(
        text.lines().collect::<Vec<_>>(),
        ["-S", "/tmp/rig-fake-rig-rs", "attach", "-t", "=one__pair"]
    );
}

/// PE-008: an unknown seat exits 1.
#[test]
fn attach_unknown_seat_exits_1() {
    let env = Env::new();
    let spec = env.write_spec("two-stub.yaml", &two_stub(&env.home));
    let (code, _, err) = json_out(
        env.rig()
            .args(["up", spec.to_str().expect("utf8"), "--json"]),
    );
    assert_eq!(code, 0, "{err}");
    let (code, _, err) = stdout(env.rig().args(["attach", "missing"]));
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("not found"), "{err}");
}

#[test]
fn gate_surfaces_as_attention() {
    let env = Env::new();
    let spec = env.write_spec("two-stub.yaml", &two_stub(&env.home));
    let (code, up, err) = json_out(
        env.rig()
            .env("RIG_STUB_SCRIPT", "print_gate=login_required")
            .args(["up", spec.to_str().expect("utf8"), "--json"]),
    );
    assert_eq!(code, 0, "{err} {up}");
    assert_eq!(up.get("status").and_then(Value::as_str), Some("attention"));
    let (code, listed, err) = json_out(env.rig().args(["ps", "--json"]));
    assert_eq!(code, 0, "{err}");
    let state = listed
        .pointer("/rigs/0/seats/0/state")
        .and_then(Value::as_str);
    let code_name = listed
        .pointer("/rigs/0/seats/0/code")
        .and_then(Value::as_str);
    assert_eq!(state, Some("attention"));
    assert_eq!(code_name, Some("login_required"));
}

#[test]
fn exit_after_ps_shows_dead() {
    let env = Env::new();
    let spec = env.write_spec(
        "one.yaml",
        &format!(
            "name: pair\nseats:\n  - name: one\n    runtime: stub\n    cwd: {}\n",
            env.home.display()
        ),
    );
    let (code, _, err) = json_out(env.rig().env("RIG_STUB_SCRIPT", "exit_after_ms=40").args([
        "up",
        spec.to_str().expect("utf8"),
        "--json",
    ]));
    assert_eq!(code, 0, "{err}");
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut state = String::new();
    while Instant::now() < deadline {
        let (code, listed, err) = json_out(env.rig().args(["ps", "--json"]));
        assert_eq!(code, 0, "{err}");
        state = listed
            .pointer("/rigs/0/seats/0/state")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if state == "dead" {
            break;
        }
        std::thread::park_timeout(Duration::from_millis(30));
    }
    assert_eq!(state, "dead");
}

#[test]
fn stub_exits_while_stdin_stays_open() {
    let mut child = std::process::Command::cargo_bin("rig-stub-agent")
        .expect("stub bin")
        .env("RIG_STUB_SCRIPT", "exit_after_ms=200")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("stub");
    let _stdin = child.stdin.take().expect("stdin");
    let started = Instant::now();
    let status = child.wait().expect("wait");
    assert!(status.success(), "{status}");
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(150),
        "exited before exit_after_ms: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "exit_after_ms waited for stdin to close: {elapsed:?}"
    );
}

#[test]
fn partial_cwd_exits_3() {
    let env = Env::new();
    let missing = env.root.path().join("missing-cwd");
    let body = format!(
        "name: pair\nseats:\n  - name: good\n    runtime: stub\n    cwd: {}\n  - name: bad\n    runtime: stub\n    cwd: {}\n",
        env.home.display(),
        missing.display()
    );
    let spec = env.write_spec("partial.yaml", &body);
    let (code, up, err) = json_out(
        env.rig()
            .args(["up", spec.to_str().expect("utf8"), "--json"]),
    );
    assert_eq!(code, 3, "{err} {up}");
    assert_eq!(up.get("status").and_then(Value::as_str), Some("partial"));
}

#[test]
fn daemon_status_does_not_start_a_daemon() {
    let env = Env::new();
    let (code, out, err) = stdout(env.rig().args(["daemon", "status"]));
    assert_eq!(code, 1, "{err} {out}");
    assert!(out.contains("daemon stopped"), "{out}");
    assert!(!env.rig_home.join("run/daemon.pid").exists(), "{err}");
    let (code, out, err) = stdout(env.rig().args(["daemon", "status", "now"]));
    assert_eq!(code, 1, "{err} {out}");
    assert!(err.contains("one subcommand"), "{err}");
    let (code, _, err) = stdout(env.rig().args(["daemon"]));
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("usage:"), "{err}");
    let (code, _, err) = stdout(env.rig().args(["daemon", "explode"]));
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("unknown daemon command"), "{err}");
    assert!(!crewmux_api::daemon_socket(&env.rig_home).exists());
}

#[test]
fn sigterm_removes_the_socket_and_pid() {
    let env = Env::new();
    let spec = env.write_spec("two-stub.yaml", &two_stub(&env.home));
    let (code, _, err) = json_out(
        env.rig()
            .args(["up", spec.to_str().expect("utf8"), "--json"]),
    );
    assert_eq!(code, 0, "{err}");
    let pid_path = env.rig_home.join("run/daemon.pid");
    let socket = crewmux_api::daemon_socket(&env.rig_home);
    let pid = std::fs::read_to_string(&pid_path).expect("pid");
    let status = std::process::Command::new("kill")
        .args(["-TERM", pid.trim()])
        .status()
        .expect("kill");
    assert!(status.success());
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && (socket.exists() || pid_path.exists()) {
        std::thread::park_timeout(Duration::from_millis(20));
    }
    assert!(!socket.exists(), "socket survived SIGTERM");
    assert!(!pid_path.exists(), "pid file survived SIGTERM");
}

#[test]
fn hermetic_home_keeps_writes_inside_the_scratch_tree() {
    let home = xtask::diff::HermeticHome::from_vars(&[(
        "PATH".to_owned(),
        std::env::var("PATH").unwrap_or_default(),
    )])
    .expect("hermetic");
    write_one(&home);
    let (code, out, err) = run_up(&home);
    assert_eq!(code, 0, "{err} {out}");
    shutdown(&home);
    assert_scratch_contains_only_known_roots(&home);
}

fn write_one(home: &xtask::diff::HermeticHome) {
    std::fs::write(
        home.home.join("two-stub.yaml"),
        format!(
            "name: pair\nseats:\n  - name: one\n    runtime: stub\n    cwd: {}\n",
            home.home.display()
        ),
    )
    .expect("spec");
}

fn run_up(home: &xtask::diff::HermeticHome) -> (i32, String, String) {
    let mut command = std::process::Command::cargo_bin("rig").expect("rig");
    command
        .args(["up", "two-stub.yaml", "--json"])
        .env_clear()
        .envs(home.child_env())
        .env("RIG_FAKE_MUX", "1")
        .current_dir(&home.home);
    stdout(&mut command)
}

fn shutdown(home: &xtask::diff::HermeticHome) {
    let socket = crewmux_api::daemon_socket(&home.rig_home());
    let client = crewmux_api::Client::over_uds(socket).with_timeout(Duration::from_secs(2));
    drop(client.request("POST", "/shutdown", Some(b"{}")));
}

fn assert_scratch_contains_only_known_roots(home: &xtask::diff::HermeticHome) {
    let scratch = home.root.path();
    let rig_home = home.rig_home();
    let socket = crewmux_api::daemon_socket(&rig_home);
    assert!(
        socket.starts_with(scratch),
        "socket left the scratch tree: {}",
        socket.display()
    );
    for path in walk(scratch) {
        let ok = path.starts_with(&rig_home)
            || path.starts_with(&home.home)
            || path.starts_with(scratch.join("bin"))
            || path.starts_with(scratch.join("tmp"))
            || path.starts_with(scratch.join("tmux-tmp"))
            || path.starts_with(scratch.join("openrig-home"))
            || path == scratch.join("tmux-argv.jsonl");
        assert!(ok, "wrote outside the scratch tree: {}", path.display());
    }
}

/// Autostart creates `RIG_HOME` and `run` at 0700 and the daemon files at 0600.
#[test]
fn autostarted_ps_keeps_private_modes() {
    let root = tempfile::TempDir::with_prefix("rig-modes-").expect("temp");
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    let rig_home = root.path().join("rig-home");
    let guard = ModeGuard {
        rig_home: rig_home.clone(),
    };
    let mut command = std::process::Command::cargo_bin("rig").expect("rig");
    command
        .args(["ps"])
        .env("HOME", &home)
        .env("RIG_HOME", &rig_home)
        .env("RIG_FAKE_MUX", "1")
        .env_remove("RIG_STUB_SCRIPT")
        .env_remove("RIG_TMUX_SOCKET")
        .env_remove("TMUX")
        .current_dir(&home);
    let (code, out, err) = stdout(&mut command);
    assert_eq!(code, 0, "{err} {out}");
    assert_eq!(mode_bits(&rig_home), 0o700, "RIG_HOME");
    assert_eq!(mode_bits(&rig_home.join("run")), 0o700, "run");
    assert_eq!(mode_bits(&rig_home.join("state")), 0o700, "state");
    assert_eq!(mode_bits(&rig_home.join("run/daemon.lock")), 0o600);
    assert_eq!(mode_bits(&rig_home.join("run/daemon.pid")), 0o600);
    assert_eq!(mode_bits(&rig_home.join("run/daemon.log")), 0o600);
    assert_eq!(mode_bits(&rig_home.join("run/daemon.start.lock")), 0o600);
    assert_eq!(mode_bits(&rig_home.join("run/daemon.sock")), 0o600);
    drop(guard);
}

struct ModeGuard {
    rig_home: std::path::PathBuf,
}

impl Drop for ModeGuard {
    fn drop(&mut self) {
        let socket = crewmux_api::daemon_socket(&self.rig_home);
        let client = crewmux_api::Client::over_uds(socket).with_timeout(Duration::from_secs(2));
        drop(client.request("POST", "/shutdown", Some(b"{}")));
    }
}

fn mode_bits(path: &std::path::Path) -> u32 {
    std::fs::metadata(path).expect("meta").permissions().mode() & 0o777
}

fn walk(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            found.push(path.clone());
            if path.is_dir() {
                pending.push(path);
            }
        }
    }
    found
}
