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
    assert!(!global_window_size_manual(&render_tmux_block()));
    for line in render_tmux_block().lines() {
        let sets_size = line.contains("window-size");
        assert!(!sets_size, "{line}");
    }
}

#[test]
fn tmux_loads_the_managed_block() {
    let tmux = which("tmux").expect("tmux is required");
    let dir = tempfile::tempdir().unwrap();
    let conf = dir.path().join("tmux.conf");
    fs::write(&conf, render_tmux_block()).unwrap();
    let socket = format!("rigrs-ac10-{}", std::process::id());
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
            "remain-on-exit",
        ],
    );
    drop(run(&tmux, &["-L", &socket, "kill-server"]));
    assert_eq!(shown.0, 0, "stderr={}", shown.2);
    assert!(
        shown.1.contains("remain-on-exit on"),
        "stdout={} stderr={}",
        shown.1,
        shown.2
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

fn run(bin: &std::path::Path, args: &[&str]) -> (i32, String, String) {
    let mut command = Command::new(bin);
    command
        .args(args)
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
