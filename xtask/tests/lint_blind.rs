//! `lint-blind-enter` allows Enter only in the dialog driver and Codex submit.

use std::fs;

use xtask::lint_blind_enter;

#[test]
fn workspace_adapters_do_not_send_blind_enter() {
    lint_blind_enter(&xtask::diff::workspace_root()).expect("lint");
}

#[test]
fn enter_outside_the_allowlist_fails() {
    let root = tempfile::tempdir().expect("temp");
    let file = root
        .path()
        .join("crates/crewmux-adapters/src/codex/other.rs");
    fs::create_dir_all(file.parent().expect("parent")).expect("dir");
    fs::write(&file, "fn bad() { let _ = Key::Enter; }\n").expect("write");
    let err = lint_blind_enter(root.path()).expect_err("enter");
    assert!(err.to_string().contains("Enter"));
}

#[test]
fn quit_and_escape_fail_even_in_the_driver() {
    let root = tempfile::tempdir().expect("temp");
    let file = root
        .path()
        .join("crates/crewmux-adapters/src/dialog/driver.rs");
    fs::create_dir_all(file.parent().expect("parent")).expect("dir");
    fs::write(&file, "fn bad() { let _ = Key::CtrlC; }\n").expect("write");
    let err = lint_blind_enter(root.path()).expect_err("ctrl-c");
    assert!(err.to_string().contains("forbidden key"));
}
