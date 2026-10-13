//! `rig context` verbs against a temp `RIG_HOME`.
#![expect(
    clippy::expect_used,
    reason = "fixture setup failures should abort the test"
)]
#![expect(
    dead_code,
    reason = "the shared CLI harness also exposes helpers this file does not call"
)]

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use common::{Env, stdout};

fn pack(dir: &Path, name: &str) {
    fs::create_dir_all(dir).expect("pack");
    fs::write(
        dir.join("manifest.yaml"),
        format!(
            "name: {name}\nversion: \"1\"\ntaxonomy: world\npurpose: fixture\nfiles:\n  - path: notes.md\n    role: notes\n"
        ),
    )
    .expect("manifest");
    fs::write(dir.join("notes.md"), "## Welcome\nhello notes\n").expect("notes");
}

#[test]
fn context_help_lists_the_library_verbs() {
    let env = Env::new();
    let (code, out, err) = stdout(env.rig().args(["context", "--help"]));
    assert_eq!(code, 0, "{err}");
    for verb in [
        "work-install",
        "trace",
        "compose",
        "list",
        "show",
        "preview",
        "get",
        "profile",
        "recap-write",
        "sync",
        "add",
        "source",
        "rm",
        "deliver",
    ] {
        assert!(out.contains(verb), "{verb} missing from\n{out}");
    }
    assert!(!out.contains("context-pack"), "{out}");
    let (unknown, _, unknown_err) = stdout(env.rig().args(["context-pack", "list"]));
    assert_ne!(unknown, 0);
    assert!(unknown_err.contains("context-pack") || unknown_err.contains("unrecognized"));
}

#[test]
fn work_install_reports_an_unconfigured_system() {
    let env = Env::new();
    let (code, out, err) = stdout(env.rig().args(["context", "work-install"]));
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("system  absent [unconfigured]"), "{out}");
    let (json_code, json_out, json_err) =
        stdout(env.rig().args(["context", "work-install", "--json"]));
    assert_eq!(json_code, 0, "{json_err}");
    let value: serde_json::Value = serde_json::from_str(json_out.trim()).expect("json");
    assert_eq!(value["systemWorld"]["state"], "absent");
}

#[test]
fn trace_reads_the_topology_tree_without_the_daemon() {
    let env = Env::new();
    let topology = env.root.path().join("topology");
    let learned = topology.join("rigs").join("demo").join("LEARNED.md");
    fs::create_dir_all(learned.parent().expect("parent")).expect("dirs");
    fs::write(&learned, "chain body\n").expect("file");
    let (code, out, err) = stdout(env.rig().env("RIG_TOPOLOGY_ROOT", &topology).args([
        "context",
        "trace",
        "--rig",
        "demo",
        "--name",
        "LEARNED.md",
    ]));
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("chain body"), "{out}");
    assert!(!crewmux_api::daemon_socket(&env.rig_home).exists());
}

#[test]
fn add_refuses_unsafe_refs_before_creating_the_store() {
    let env = Env::new();
    let src = env.home.join("src-pack");
    pack(&src, "goodpack");
    let store = env.rig_home.join("context");
    let (code, _, err) = stdout(env.rig().args([
        "context",
        "add",
        src.to_str().expect("src"),
        "--name",
        "../escape",
    ]));
    assert_ne!(code, 0);
    assert!(err.contains("unsafe pack ref"), "{err}");
    assert!(!store.exists());
    fs::write(
        src.join("manifest.yaml"),
        "name: goodpack\nversion: '1:0:0'\ntaxonomy: world\nfiles:\n  - path: notes.md\n    role: notes\n",
    )
    .expect("bad version");
    let (version_code, _, version_err) =
        stdout(
            env.rig()
                .args(["context", "add", src.to_str().expect("src")]),
        );
    assert_ne!(version_code, 0);
    assert!(version_err.contains("version"), "{version_err}");
    assert!(!store.exists());
    let (git_code, _, git_err) = stdout(env.rig().args(["context", "source", "inspect", "alpha"]));
    assert_ne!(git_code, 0);
    assert!(
        git_err.contains("Git source selection is not available"),
        "{git_err}"
    );
}

#[test]
fn library_verbs_list_show_compose_and_remove() {
    let env = Env::new();
    let src = env.home.join("src-pack");
    pack(&src, "goodpack");
    let (added, add_out, add_err) =
        stdout(
            env.rig()
                .args(["context", "add", src.to_str().expect("src")]),
        );
    assert_eq!(added, 0, "{add_err}\n{add_out}");
    assert!(
        env.rig_home
            .join("context")
            .join("goodpack")
            .join("manifest.yaml")
            .is_file()
    );
    let (listed, list_out, list_err) = stdout(env.rig().args(["context", "list", "--json"]));
    assert_eq!(listed, 0, "{list_err}");
    assert!(list_out.contains("goodpack"), "{list_out}");
    let (shown, show_out, show_err) = stdout(env.rig().args(["context", "show", "goodpack"]));
    assert_eq!(shown, 0, "{show_err}");
    assert!(show_out.contains("goodpack"), "{show_out}");
    let (previewed, preview_out, preview_err) =
        stdout(env.rig().args(["context", "preview", "goodpack"]));
    assert_eq!(previewed, 0, "{preview_err}");
    assert!(
        preview_out.contains("hello notes") || preview_out.contains("Welcome"),
        "{preview_out}"
    );
    let note = env.home.join("extra.md");
    fs::write(&note, "extra line\n").expect("extra");
    let (composed, compose_out, compose_err) = stdout(env.rig().args([
        "context",
        "compose",
        "--out",
        "packs/brief",
        "--from",
        note.to_str().expect("note"),
    ]));
    assert_eq!(composed, 0, "{compose_err}\n{compose_out}");
    assert!(compose_out.contains("packs/brief"), "{compose_out}");
    let (removed, remove_out, remove_err) = stdout(env.rig().args(["context", "rm", "goodpack"]));
    assert_eq!(removed, 0, "{remove_err}\n{remove_out}");
    assert!(remove_out.contains("Removed"), "{remove_out}");
}

#[test]
fn get_sync_and_profile_round_trip() {
    let env = Env::new();
    let src = env.home.join("src-pack");
    pack(&src, "goodpack");
    let (added, _, add_err) =
        stdout(
            env.rig()
                .args(["context", "add", src.to_str().expect("src")]),
        );
    assert_eq!(added, 0, "{add_err}");
    let (got, get_out, get_err) = stdout(env.rig().args(["context", "get", "goodpack"]));
    assert_eq!(got, 0, "{get_err}\n{get_out}");
    assert!(get_out.contains("hello notes"), "{get_out}");
    let (synced, sync_out, sync_err) = stdout(env.rig().args(["context", "sync"]));
    assert_eq!(synced, 0, "{sync_err}\n{sync_out}");
    assert!(sync_out.contains("Indexed"), "{sync_out}");
    let (profiled, profile_out, profile_err) =
        stdout(
            env.rig()
                .args(["context", "profile", "goodpack", "--situation", "fresh"]),
        );
    assert_ne!(profiled, 0, "{profile_out}");
    assert!(profile_err.contains("declares no atoms"), "{profile_err}");
    let (startup, _, startup_err) = stdout(env.rig().args([
        "context", "deliver", "--ref", "goodpack", "--seat", "one", "--mode", "startup",
    ]));
    assert_ne!(startup, 0);
    assert!(startup_err.contains("not implemented"), "{startup_err}");
}

#[test]
fn recap_write_and_deliver_speak_the_daemon() {
    let env = Env::new();
    let topology = env.root.path().join("topology");
    fs::create_dir_all(topology.join("rigs").join("demo")).expect("rig");
    let file = env.home.join("recap.md");
    fs::write(&file, "## Notes\nconclusions only\n").expect("recap");
    let (code, out, err) = stdout(env.rig().env("RIG_TOPOLOGY_ROOT", &topology).args([
        "context",
        "recap-write",
        "--rig",
        "demo",
        "--seat",
        "s1",
        "--file",
        file.to_str().expect("file"),
    ]));
    assert_eq!(code, 0, "{err}\n{out}");
    assert!(
        topology
            .join("rigs")
            .join("demo")
            .join("seats")
            .join("s1")
            .join("RECAP.md")
            .is_file()
    );
    let (missing, _, deliver_err) = stdout(env.rig().args([
        "context",
        "deliver",
        "--ref",
        "missing-pack",
        "--seat",
        "one",
        "--mode",
        "paste",
    ]));
    assert_ne!(missing, 0);
    assert!(deliver_err.contains("not found"), "{deliver_err}");
}

#[test]
fn add_uses_the_parsed_manifest_name() {
    let env = Env::new();
    let src = env.home.join("different-source-name");
    fs::create_dir_all(&src).expect("src");
    fs::write(
        src.join("manifest.yaml"),
        "name: >-\n  desired\nversion: \"1\"\ntaxonomy: world\npurpose: fixture\nfiles:\n  - path: notes.md\n    role: notes\n",
    )
    .expect("manifest");
    fs::write(src.join("notes.md"), "preserved bytes").expect("notes");
    let (code, out, err) =
        stdout(
            env.rig()
                .args(["context", "add", src.to_str().expect("src"), "--json"]),
        );
    assert_eq!(code, 0, "{err}\n{out}");
    let value: serde_json::Value = serde_json::from_str(out.trim()).expect("json");
    let installed = value["installedAt"].as_str().unwrap_or("");
    assert!(installed.ends_with("/desired"), "{installed}");
    assert_eq!(
        fs::read_to_string(
            env.rig_home
                .join("context")
                .join("desired")
                .join("notes.md")
        )
        .expect("notes"),
        "preserved bytes"
    );
}

#[test]
fn add_drops_a_partial_install_when_a_member_is_a_symlink() {
    let env = Env::new();
    let src = env.home.join("src-pack");
    pack(&src, "goodpack");
    std::os::unix::fs::symlink(src.join("notes.md"), src.join("leak.md")).expect("symlink");
    let (code, _, err) = stdout(
        env.rig()
            .args(["context", "add", src.to_str().expect("src")]),
    );
    assert_ne!(code, 0);
    assert!(err.contains("symlink"), "{err}");
    assert!(!env.rig_home.join("context").join("goodpack").exists());
    let store = env.rig_home.join("context");
    if store.is_dir() {
        for entry in fs::read_dir(&store).expect("store") {
            let name = entry.expect("entry").file_name();
            assert!(
                !name.to_string_lossy().starts_with(".tmp-add-"),
                "{}",
                name.to_string_lossy()
            );
        }
    }
}

#[test]
fn get_address_fork_serves_a_header() {
    let env = Env::new();
    let src = env.home.join("src-pack");
    pack(&src, "goodpack");
    let (added, _, add_err) =
        stdout(
            env.rig()
                .args(["context", "add", src.to_str().expect("src")]),
        );
    assert_eq!(added, 0, "{add_err}");
    let (code, out, err) = stdout(
        env.rig()
            .args(["context", "get", "goodpack/notes.md#welcome"]),
    );
    assert_eq!(code, 0, "{err}\n{out}");
    assert!(out.contains("hello notes"), "{out}");
}

#[test]
fn setup_links_openrig_only_when_the_workspace_is_set() {
    let env = Env::new();
    let workspace = env.root.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let bin = env.root.path().join("bin");
    fs::create_dir_all(&bin).expect("bin");
    let script = "#!/bin/sh\nexit 0\n";
    for name in ["sh", "bash", "curl", "brew"] {
        let path = bin.join(name);
        fs::write(&path, script).expect("shim");
        let mut mode = fs::metadata(&path).expect("meta").permissions();
        mode.set_mode(0o755);
        fs::set_permissions(&path, mode).expect("mode");
    }
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let (code, _, err) = stdout(
        env.rig()
            .env("PATH", &path)
            .env("RIG_WORKSPACE", &workspace)
            .args([
                "setup",
                "--providers",
                "claude",
                "--no-herdr",
                "--no-ghostty",
            ]),
    );
    assert_eq!(code, 0, "{err}");
    let link = workspace.join(".openrig");
    let target = fs::read_link(&link).expect("symlink");
    assert_eq!(target, std::path::PathBuf::from(".rig-rs"));
    let (again, _, again_err) = stdout(
        env.rig()
            .env("PATH", &path)
            .env("RIG_WORKSPACE", &workspace)
            .args([
                "setup",
                "--providers",
                "claude",
                "--no-herdr",
                "--no-ghostty",
            ]),
    );
    assert_eq!(again, 0, "{again_err}");
    assert_eq!(
        fs::read_link(&link).expect("still"),
        std::path::PathBuf::from(".rig-rs")
    );
}
