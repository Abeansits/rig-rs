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
fn planted_newline_into_and_const_fail() {
    for (label, body) in [
        ("nl", "fn sneak() { send(Key::Literal(\"\\n\")); }\n"),
        ("hex", "fn sneak() { send(Key::Literal(\"\\x0d\")); }\n"),
        ("uni", "fn sneak() { send(Key::Literal(\"\\u{d}\")); }\n"),
        ("into", "fn sneak() { send(\"\\r\".into()); }\n"),
        (
            "concat",
            "fn sneak() { send(Key::Literal(concat!(\"\\r\", \"\"))); }\n",
        ),
        (
            "const",
            "const CR: &str = \"\\r\";\nfn sneak() { send(Key::Literal(CR)); }\n",
        ),
        ("split", "fn sneak() { send(Key::\nEnter); }\n"),
        (
            "glob",
            "use crewmux_types::Key::*;\nfn sneak() { send(Enter); }\n",
        ),
        ("cm", "fn sneak() { args.push(\"C-m\"); }\n"),
        ("word", "fn sneak() { args.push(\"Enter\"); }\n"),
    ] {
        let root = scratch(label);
        let path = root.join("crates/crewmux-adapters/src/claude/sneak.rs");
        write(&path, body).expect("write");
        let err = lint_blind_enter(&root).expect_err(label);
        assert!(err.to_string().contains("Enter"), "{label} {err}");
        drop(fs::remove_dir_all(&root));
    }
}

#[test]
fn string_comment_does_not_hide_a_later_enter() {
    let root = scratch("url");
    let path = root.join("crates/crewmux-adapters/src/claude/sneak.rs");
    write(
        &path,
        "fn sneak() { let _ = \"http://example\"; send(Key::Enter); }\n",
    )
    .expect("write");
    let err = lint_blind_enter(&root).expect_err("enter");
    assert!(err.to_string().contains("Enter"), "{err}");
    drop(fs::remove_dir_all(&root));
}

#[test]
fn a_real_comment_still_hides_enter() {
    let root = scratch("comment");
    let path = root.join("crates/crewmux-adapters/src/claude/screen.rs");
    write(
        &path,
        "fn go() { let _ = \"http://example\"; // send(Key::Enter)\n}\n",
    )
    .expect("write");
    lint_blind_enter(&root).expect("comment");
    drop(fs::remove_dir_all(&root));
}

#[test]
fn char_from_cr_and_bare_cr_paste_fail() {
    for (label, body) in [
        ("from", "fn sneak() { let _ = char::from(13); }\n"),
        ("from-hex", "fn sneak() { let _ = char::from(0x0d); }\n"),
        ("paste", "fn sneak() { paste(\"\\r\"); }\n"),
        ("held", "fn sneak() { let cr = \"\\r\"; }\n"),
    ] {
        let root = scratch(label);
        let path = root.join("crates/crewmux-adapters/src/claude/sneak.rs");
        write(&path, body).expect("write");
        let err = lint_blind_enter(&root).expect_err(label);
        assert!(err.to_string().contains("Enter"), "{label} {err}");
        drop(fs::remove_dir_all(&root));
    }
    let root = scratch("from-submit");
    let path = root.join("crates/crewmux-adapters/src/claude/submit.rs");
    write(&path, "fn go() { let _ = char::from(13); }\n").expect("write");
    let err = lint_blind_enter(&root).expect_err("from-submit");
    assert!(err.to_string().contains("Enter"), "{err}");
    drop(fs::remove_dir_all(&root));
    let root = scratch("thirteen");
    let path = root.join("crates/crewmux-adapters/src/claude/sneak.rs");
    write(&path, "fn sneak() { let _ = char::from(130); }\n").expect("write");
    lint_blind_enter(&root).expect("130");
    drop(fs::remove_dir_all(&root));
}

#[test]
fn submit_mode_enter_is_not_a_key() {
    let root = scratch("mode");
    let path = root.join("crates/crewmux-adapters/src/claude/screen.rs");
    write(
        &path,
        "fn go(mode: SubmitMode) { let _ = mode == SubmitMode::Enter; }\n",
    )
    .expect("write");
    lint_blind_enter(&root).expect("mode");
    drop(fs::remove_dir_all(&root));
}

#[test]
fn embedded_carriage_return_shapes_fail() {
    for (label, body) in [
        ("embed", "fn sneak() { paste(\"a\\rb\"); }\n"),
        ("as-char", "fn sneak() { let c = 13 as char; }\n"),
        ("hex", "fn sneak() { let c = 0x0d; }\n"),
        ("hex-as", "fn sneak() { let c = 0x0d as char; }\n"),
        ("u32", "fn sneak() { let c = char::from_u32(13); }\n"),
    ] {
        let root = scratch(label);
        let path = root.join("crates/crewmux-adapters/src/claude/sneak.rs");
        write(&path, body).expect("write");
        let err = lint_blind_enter(&root).expect_err(label);
        assert!(err.to_string().contains("Enter"), "{label} {err}");
        drop(fs::remove_dir_all(&root));
    }
}

#[test]
fn literal_cr_byte_in_paste_text_fails() {
    let root = scratch("cr-byte");
    let path = root.join("crates/crewmux-adapters/src/claude/sneak.rs");
    let mut body = String::from("fn sneak() { paste(\"a");
    body.push('\r');
    body.push_str("b\"); }\n");
    write(&path, &body).expect("write");
    let err = lint_blind_enter(&root).expect_err("cr-byte");
    assert!(err.to_string().contains("Enter"), "{err}");
    drop(fs::remove_dir_all(&root));
}

#[test]
fn from_u32_in_submit_fails() {
    let root = scratch("u32-submit");
    let path = root.join("crates/crewmux-adapters/src/claude/submit.rs");
    write(&path, "fn go() { let _ = char::from_u32(13); }\n").expect("write");
    let err = lint_blind_enter(&root).expect_err("u32");
    assert!(err.to_string().contains("Enter"), "{err}");
    drop(fs::remove_dir_all(&root));
}

#[test]
fn raw_string_with_slashes_does_not_hide_a_later_enter() {
    let root = scratch("raw");
    let path = root.join("crates/crewmux-adapters/src/claude/sneak.rs");
    write(
        &path,
        "fn sneak() { let _ = r##\"http://example \" // still the string\"##; send(Key::Enter); }\n",
    )
    .expect("write");
    let err = lint_blind_enter(&root).expect_err("raw");
    assert!(err.to_string().contains("Enter"), "{err}");
    drop(fs::remove_dir_all(&root));
}

#[test]
fn a_comment_after_a_closed_raw_string_still_hides_enter() {
    let root = scratch("raw-comment");
    let path = root.join("crates/crewmux-adapters/src/claude/screen.rs");
    write(
        &path,
        "fn go() { let _ = r##\"http://example // inside\"##; // send(Key::Enter)\n}\n",
    )
    .expect("write");
    lint_blind_enter(&root).expect("comment");
    drop(fs::remove_dir_all(&root));
}

#[test]
fn nearby_integers_are_not_a_carriage_return() {
    for (label, body) in [
        ("u32-130", "fn sneak() { let _ = char::from_u32(130); }\n"),
        ("as-130", "fn sneak() { let _ = 130 as char; }\n"),
        ("hex-long", "fn sneak() { let _ = 0x0d0; }\n"),
        ("from-130", "fn sneak() { let _ = char::from(130); }\n"),
    ] {
        let root = scratch(label);
        let path = root.join("crates/crewmux-adapters/src/claude/sneak.rs");
        write(&path, body).expect("write");
        lint_blind_enter(&root).expect(label);
        drop(fs::remove_dir_all(&root));
    }
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
