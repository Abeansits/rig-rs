//! `lint-mux` catches attachment-format bypasses, shell variants, and global window-size.

use std::fs;
use std::path::{Path, PathBuf};

use xtask::{lint_mux_at, lint_workspace_root};

fn needle() -> String {
    String::from_utf8(vec![
        115, 101, 115, 115, 105, 111, 110, 95, 97, 116, 116, 97, 99, 104, 101, 100,
    ])
    .unwrap_or_default()
}

fn scratch(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("xtask-lint-{label}-{}", std::process::id()));
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
fn workspace_mux_sources_pass() {
    lint_mux_at(&lint_workspace_root()).expect("lint");
}

#[test]
fn a_code_mention_outside_the_doc_comment_fails() {
    let root = scratch("mention");
    let clients = root.join("crates/crewmux-mux/src/clients.rs");
    let word = needle();
    write(&clients, &format!("//! mention {word} only here\n")).expect("write");
    lint_mux_at(&root).expect("docs are allowed");
    let bad = root.join("crates/crewmux-mux/src/other.rs");
    write(&bad, &format!("fn probe() {{ let _ = \"{word}\"; }}\n")).expect("write");
    let err = lint_mux_at(&root).expect_err("code mention");
    assert!(err.contains("other.rs"), "{err}");
    drop(fs::remove_dir_all(&root));
}

#[test]
fn concat_and_adjacent_strings_cannot_hide_the_format() {
    let root = scratch("join");
    let word = needle();
    let mid = word.len() / 2;
    let (left, right) = word.split_at(mid);
    let keyword = ["conc", "at!"].join("");
    let file = root.join("crates/crewmux-mux/src/hidden.rs");
    write(
        &file,
        &format!("fn probe() {{ let _ = {keyword}(\"{left}\", \"{right}\"); }}\n"),
    )
    .expect("write");
    let err = lint_mux_at(&root).expect_err("concat");
    assert!(err.contains("hidden.rs"), "{err}");
    write(
        &file,
        &format!("fn probe() {{ let _ = \"{left}\" \"{right}\"; }}\n"),
    )
    .expect("write");
    let err = lint_mux_at(&root).expect_err("adjacent");
    assert!(err.contains("hidden.rs"), "{err}");
    drop(fs::remove_dir_all(&root));
}

#[test]
fn shell_command_variants_in_mux_fail() {
    let root = scratch("shell");
    let file = root.join("crates/crewmux-mux/src/exec.rs");
    let cases = [
        "fn probe() { std::process::Command::new(\"sh\"); }\n",
        "fn probe() { std::process::Command :: new ( \"/bin/bash\" ); }\n",
        "fn probe() { std::process::Command::new(\"/usr/bin/zsh\"); }\n",
        "fn probe() { let shell = \"dash\"; std::process::Command::new(shell); }\n",
        "fn probe() { let bin = \"/usr/bin/ksh\"; Command::new(bin); }\n",
    ];
    for source in cases {
        write(&file, source).expect("write");
        let err = lint_mux_at(&root).expect_err(source);
        assert!(err.contains("exec.rs"), "{err} for {source}");
    }
    write(
        &file,
        "fn probe() { std::process::Command::new(\"tmux\"); }\n",
    )
    .expect("write");
    lint_mux_at(&root).expect("tmux is not a shell");
    drop(fs::remove_dir_all(&root));
}

#[test]
fn global_window_size_is_rejected_in_strings_and_other_files() {
    let root = scratch("size");
    let key = ["window", "size"].join("-");
    let flag = format!("-{}", "g");
    let rust = root.join("crates/crewmux-mux/src/argv.rs");
    write(
        &rust,
        &format!("// set-option {flag} {key} stays a comment\n"),
    )
    .expect("write");
    lint_mux_at(&root).expect("comments are allowed in rust");
    write(
        &rust,
        &format!("fn probe() {{ let _ = \"set-option {flag} {key} manual\"; }}\n"),
    )
    .expect("write");
    let err = lint_mux_at(&root).expect_err("string");
    assert!(err.contains("argv.rs"), "{err}");
    let keyword = ["conc", "at!"].join("");
    write(
        &rust,
        &format!("fn probe() {{ let _ = {keyword}(\"set-option {flag} \", \"{key}\"); }}\n"),
    )
    .expect("write");
    let err = lint_mux_at(&root).expect_err("joined strings");
    assert!(err.contains("argv.rs"), "{err}");
    let notes = root.join("notes.md");
    write(&notes, &format!("set-option {flag} {key} manual\n")).expect("write");
    write(&rust, "fn probe() {}\n").expect("write");
    let err = lint_mux_at(&root).expect_err("markdown");
    assert!(err.contains("notes.md"), "{err}");
    drop(fs::remove_dir_all(&root));
}

#[test]
fn argv_tokens_in_an_array_cannot_hide_a_global_window_size() {
    let root = scratch("argv");
    let rust = root.join("crates/crewmux-mux/src/argv.rs");
    let key = ["window", "size"].join("-");
    let flag = format!("-{}", "g");
    let owned = format!(
        "fn probe() {{ let _ = vec![\"set-option\".to_owned(), \"{flag}\".to_owned(), \"{key}\".to_owned()]; }}\n"
    );
    write(&rust, &owned).expect("write");
    let err = lint_mux_at(&root).expect_err("vec");
    assert!(err.contains("argv.rs"), "{err}");
    let array =
        format!("fn probe() {{ let _ = &[\"set-option\", \"{flag}\", \"{key}\", \"latest\"]; }}\n");
    write(&rust, &array).expect("write");
    let err = lint_mux_at(&root).expect_err("array");
    assert!(err.contains("argv.rs"), "{err}");
    let session = format!(
        "fn probe() {{ let _ = vec![\"set-option\".to_owned(), \"-t\".to_owned(), \"{key}\".to_owned()]; }}\n"
    );
    write(&rust, &session).expect("write");
    lint_mux_at(&root).expect("session scope is allowed");
    drop(fs::remove_dir_all(&root));
}
