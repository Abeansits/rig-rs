//! `lint-blind-enter` rejects Enter outside submit and the dialog driver.

use std::fs;
use std::path::{Path, PathBuf};

use xtask::{lint_blind_enter, lint_workspace_root};

fn scratch(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("xtask-enter-{label}-{}", std::process::id()));
    drop(fs::remove_dir_all(&root));
    root
}

fn write(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, text)
}

#[test]
fn workspace_adapter_sources_pass() {
    lint_blind_enter(&lint_workspace_root()).expect("lint");
}

#[test]
fn planted_enter_outside_the_driver_fails() {
    let root = scratch("enter");
    let bad = root.join("crates/crewmux-adapters/src/claude/bad.rs");
    write(&bad, "fn sneak() { send(Key::Enter); }\n").expect("write");
    let err = lint_blind_enter(&root).expect_err("enter");
    assert!(err.to_string().contains("Enter"), "{err}");
    drop(fs::remove_dir_all(&root));
}

#[test]
fn planted_quit_key_in_submit_fails() {
    let root = scratch("quit");
    let path = root.join("crates/crewmux-adapters/src/claude/submit.rs");
    write(&path, "fn stop() { send(Key::CtrlC); }\n").expect("write");
    let err = lint_blind_enter(&root).expect_err("quit");
    assert!(err.to_string().contains("quit"), "{err}");
    drop(fs::remove_dir_all(&root));
}

#[test]
fn planted_carriage_return_literal_fails() {
    let root = scratch("cr");
    let path = root.join("crates/crewmux-adapters/src/claude/sneak.rs");
    write(&path, "fn sneak() { send(Key::Literal(\"\\r\")); }\n").expect("write");
    let err = lint_blind_enter(&root).expect_err("cr");
    assert!(err.to_string().contains("Enter"), "{err}");
    drop(fs::remove_dir_all(&root));
}

#[test]
fn carriage_return_inside_submit_fails() {
    let root = scratch("cr-submit");
    let path = root.join("crates/crewmux-adapters/src/claude/submit.rs");
    write(&path, "fn go() { send(Key::Literal(\"\\r\")); }\n").expect("write");
    let err = lint_blind_enter(&root).expect_err("cr");
    assert!(err.to_string().contains("Enter"), "{err}");
    drop(fs::remove_dir_all(&root));
}

#[test]
fn planted_digit_literal_fails() {
    let root = scratch("digit");
    let path = root.join("crates/crewmux-adapters/src/dialog/drive.rs");
    write(&path, "fn pick() { send(Key::Literal(\"2\")); }\n").expect("write");
    let err = lint_blind_enter(&root).expect_err("digit");
    assert!(err.to_string().contains("menu literal"), "{err}");
    drop(fs::remove_dir_all(&root));
}
