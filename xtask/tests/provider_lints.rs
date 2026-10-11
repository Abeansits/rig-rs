//! The Claude user-config lint and the tmux window-size lint.

use std::fs;

use xtask::{lint_claude_json, lint_window_size};

fn root() -> std::path::PathBuf {
    xtask::diff::workspace_root()
}

#[test]
fn workspace_claude_json_lint_passes() {
    lint_claude_json(&root()).unwrap();
}

#[test]
fn workspace_window_size_lint_passes() {
    lint_window_size(&root()).unwrap();
}

#[test]
fn planted_claude_basename_fails() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    let banned = concat!(".claude", ".json");
    fs::write(
        src.join("other.rs"),
        format!("const NAME: &str = \"{banned}\";\n"),
    )
    .unwrap();
    let error = lint_claude_json(dir.path()).unwrap_err();
    assert!(error.to_string().contains("lint-claude-json"), "{error}");
}

#[test]
fn planted_window_size_fails() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("crates/crewmux-provider/src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("bad.rs"), "set -g window-size manual\n").unwrap();
    let error = lint_window_size(dir.path()).unwrap_err();
    assert!(error.to_string().contains("lint-window-size"), "{error}");
}
