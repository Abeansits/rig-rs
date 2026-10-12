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

#[test]
fn carriage_return_literal_fails_inside_submit() {
    let root = tempfile::tempdir().expect("temp");
    let file = root
        .path()
        .join("crates/crewmux-adapters/src/codex/submit.rs");
    fs::create_dir_all(file.parent().expect("parent")).expect("dir");
    fs::write(&file, "fn bad() { let _ = Key::Literal(\"\\r\"); }\n").expect("write");
    let err = lint_blind_enter(root.path()).expect_err("cr");
    assert!(err.to_string().contains("carriage return"), "{err}");
}

#[test]
fn multiline_enter_outside_the_allowlist_fails() {
    let root = scratch(
        "crates/crewmux-adapters/src/codex/other.rs",
        "fn bad() { let _ = Key::\nEnter; }\n",
    );
    let err = lint_blind_enter(root.path()).expect_err("enter");
    assert!(err.to_string().contains("Enter"), "{err}");
}

#[test]
fn multiline_carriage_return_fails_inside_submit() {
    let root = scratch(
        "crates/crewmux-adapters/src/codex/submit.rs",
        "fn bad() { let _ = Key::Literal(\n\"\\r\"\n); }\n",
    );
    let err = lint_blind_enter(root.path()).expect_err("cr");
    assert!(err.to_string().contains("carriage return"), "{err}");
}

#[test]
fn aliased_enter_outside_the_allowlist_fails() {
    let root = scratch(
        "crates/crewmux-adapters/src/codex/other.rs",
        "use crewmux_types::Key as K;\nfn bad() { let _ = K::Enter; }\n",
    );
    let err = lint_blind_enter(root.path()).expect_err("alias");
    assert!(err.to_string().contains("Enter"), "{err}");
}

#[test]
fn aliased_ctrl_c_fails_in_the_driver() {
    let root = scratch(
        "crates/crewmux-adapters/src/dialog/driver.rs",
        "type K = Key;\nfn bad() { let _ = K::CtrlC; }\n",
    );
    let err = lint_blind_enter(root.path()).expect_err("ctrl-c");
    assert!(err.to_string().contains("forbidden key"), "{err}");
}

#[test]
fn raw_c_m_fails_inside_submit() {
    let root = scratch(
        "crates/crewmux-adapters/src/codex/submit.rs",
        "fn bad() { let keys = \"C-m\"; let _ = keys; }\n",
    );
    let err = lint_blind_enter(root.path()).expect_err("c-m");
    assert!(err.to_string().contains("C-m"), "{err}");
}

#[expect(
    clippy::expect_used,
    reason = "a missing temp directory fails the test"
)]
fn scratch(rel: &str, body: &str) -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("temp");
    let file = root.path().join(rel);
    fs::create_dir_all(file.parent().expect("parent")).expect("dir");
    fs::write(&file, body).expect("write");
    root
}
