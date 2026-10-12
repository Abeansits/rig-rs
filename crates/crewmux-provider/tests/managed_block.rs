//! tmux managed block: idempotent, removable, and loadable. No global window size.
#![expect(
    clippy::unwrap_used,
    reason = "the tmux load test asserts process output directly"
)]

use std::fs;
use std::process::{Command, Stdio};

use crewmux_provider::{
    ManagedBlock, TMUX_BLOCK_BODY, global_window_size_manual, managed_block_current,
    render_tmux_block,
};

#[test]
fn upsert_is_idempotent_and_remove_is_exact() {
    let outside_before = "set -g status on\n";
    let outside_after = "set -g status-right \"ok\"\n";
    let original = format!("{outside_before}{outside_after}");
    let once = ManagedBlock::TMUX
        .upsert(&original, TMUX_BLOCK_BODY)
        .unwrap();
    let twice = ManagedBlock::TMUX.upsert(&once, TMUX_BLOCK_BODY).unwrap();
    assert_eq!(once, twice);
    assert!(managed_block_current(&twice));
    assert!(twice.contains(outside_before));
    assert!(twice.contains(outside_after));
    let removed = ManagedBlock::TMUX.remove(&twice);
    assert_eq!(removed, original);
    let stale = ManagedBlock::TMUX.upsert("tail\n", "stale-body\n").unwrap();
    let doubled = format!("{once}{stale}");
    let replaced = ManagedBlock::TMUX
        .upsert(&doubled, TMUX_BLOCK_BODY)
        .unwrap();
    assert_eq!(replaced.matches("# >>> rig-rs managed >>>").count(), 1);
    assert!(!replaced.contains("stale-body"));
    assert!(replaced.contains("tail"));
    assert!(replaced.contains(outside_after));
    let collapsed_again = ManagedBlock::TMUX
        .upsert(&replaced, TMUX_BLOCK_BODY)
        .unwrap();
    assert_eq!(collapsed_again, replaced);
    let crlf = once.replace('\n', "\r\n");
    let crlf_next = ManagedBlock::TMUX.upsert(&crlf, TMUX_BLOCK_BODY).unwrap();
    assert!(crlf_next.contains("\r\n"));
    assert_eq!(
        ManagedBlock::TMUX.remove(&crlf_next).replace("\r\n", "\n"),
        original
    );
    assert!(!global_window_size_manual(&render_tmux_block()));
    assert!(!render_tmux_block().contains("remain-on-exit"));
    for line in render_tmux_block().lines() {
        let sets_size = line.contains("window-size");
        assert!(!sets_size, "{line}");
    }
}

#[test]
#[cfg_attr(
    target_os = "macos",
    ignore = "tmux is absent from the macOS cargo test job; this is not a pass. Nextest installs tmux. RIG_REQUIRE_TMUX fails the load when the binary is missing"
)]
fn tmux_loads_the_managed_block() {
    let Some(tmux) = which("tmux") else {
        panic!("tmux is not on PATH");
    };
    sweep_ac10(&tmux);
    let dir = tempfile::tempdir().unwrap();
    let conf = dir.path().join("tmux.conf");
    fs::write(&conf, render_tmux_block()).unwrap();
    let socket = format!("rigrs-ac10-{}", std::process::id());
    let _guard = Ac10Server {
        bin: tmux.clone(),
        socket: socket.clone(),
    };
    let shown = run(
        &tmux,
        &[
            "-L",
            &socket,
            "-f",
            conf.to_str().unwrap(),
            "start-server",
            ";",
            "show",
            "-g",
            "mouse",
        ],
    );
    assert_eq!(shown.0, 0, "stderr={}", shown.2);
    assert!(
        shown.1.contains("mouse on"),
        "stdout={} stderr={}",
        shown.1,
        shown.2
    );
}

#[test]
fn missing_tmux_fails_when_required() {
    if std::env::var_os("RIG_REQUIRE_TMUX").is_none() {
        return;
    }
    assert!(
        which("tmux").is_some(),
        "RIG_REQUIRE_TMUX is set and tmux is not on PATH"
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

struct Ac10Server {
    bin: std::path::PathBuf,
    socket: String,
}

impl Drop for Ac10Server {
    fn drop(&mut self) {
        kill_ac10(&self.bin, &self.socket);
    }
}

fn sweep_ac10(bin: &std::path::Path) {
    let dir = std::path::PathBuf::from(format!("/tmp/tmux-{}", rustix::process::getuid().as_raw()));
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    let me = std::process::id();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(rest) = name.strip_prefix("rigrs-ac10-") else {
            continue;
        };
        if rest != me.to_string() {
            continue;
        }
        kill_ac10(bin, name);
    }
}

fn kill_ac10(bin: &std::path::Path, name: &str) {
    let mine = format!("rigrs-ac10-{}", std::process::id());
    if name != mine {
        return;
    }
    drop(run(bin, &["-L", name, "-f", "/dev/null", "kill-server"]));
    let path = std::path::PathBuf::from(format!(
        "/tmp/tmux-{}/{name}",
        rustix::process::getuid().as_raw()
    ));
    drop(fs::remove_file(path));
}

fn run(bin: &std::path::Path, args: &[&str]) -> (i32, String, String) {
    let mut command = Command::new(bin);
    command
        .args(args)
        .env_remove("TMUX")
        .env_remove("TMUX_TMPDIR")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command.spawn().unwrap();
    let output = child.wait_with_output().unwrap();
    (
        output.status.code().unwrap_or(1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}
