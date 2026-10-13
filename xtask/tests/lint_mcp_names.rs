//! AC13–AC14: legacy MCP names and the MCP crate's dependency edge.

use std::fs;

use xtask::{lint_mcp_legacy_names, lint_mcp_state_path};

fn root() -> std::path::PathBuf {
    xtask::diff::workspace_root()
}

#[test]
fn workspace_legacy_names_pass() {
    lint_mcp_legacy_names(&root()).unwrap();
}

#[test]
fn workspace_mcp_state_path_passes() {
    lint_mcp_state_path(&root()).unwrap();
}

#[test]
fn planted_legacy_name_in_docs_fails() {
    let dir = tempfile::tempdir().unwrap();
    let docs = dir.path().join("docs");
    fs::create_dir_all(&docs).unwrap();
    fs::write(docs.join("tools.md"), "Call rig_send from the seat.\n").unwrap();
    let error = lint_mcp_legacy_names(dir.path()).unwrap_err();
    assert!(
        error.to_string().contains("lint-mcp-legacy-names"),
        "{error}"
    );
    assert!(error.to_string().contains("rig_send"), "{error}");
}

#[test]
fn planted_legacy_name_in_crate_src_fails() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("crates/example/src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("lib.rs"), "const OLD: &str = \"rig_up\";\n").unwrap();
    let error = lint_mcp_legacy_names(dir.path()).unwrap_err();
    assert!(error.to_string().contains("rig_up"), "{error}");
}

#[test]
fn names_rs_is_allow_listed() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("crates/crewmux-mcp/src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("names.rs"),
        "pub const LEGACY: &str = \"rig_send\";\n",
    )
    .unwrap();
    fs::write(src.join("other.rs"), "fn rig_down_handler() {}\n").unwrap();
    lint_mcp_legacy_names(dir.path()).unwrap();
}

#[test]
fn skill_file_with_a_legacy_name_fails() {
    let dir = tempfile::tempdir().unwrap();
    let skill = dir.path().join("assets/skills/rig");
    fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("SKILL.md"), "rig_capture is gone\n").unwrap();
    let error = lint_mcp_legacy_names(dir.path()).unwrap_err();
    assert!(error.to_string().contains("rig_capture"), "{error}");
}

#[test]
fn planted_state_dependency_fails() {
    let dir = tempfile::tempdir().unwrap();
    let crate_dir = dir.path().join("crates/crewmux-mcp");
    fs::create_dir_all(&crate_dir).unwrap();
    fs::write(
        crate_dir.join("Cargo.toml"),
        "[dependencies]\ncrewmux-store.workspace = true\n",
    )
    .unwrap();
    let error = lint_mcp_state_path(dir.path()).unwrap_err();
    assert!(error.to_string().contains("lint-mcp-state-path"), "{error}");
    assert!(error.to_string().contains("crewmux-store"), "{error}");
}

#[test]
fn dev_dependency_on_a_state_crate_passes() {
    let dir = tempfile::tempdir().unwrap();
    let crate_dir = dir.path().join("crates/crewmux-mcp");
    fs::create_dir_all(&crate_dir).unwrap();
    fs::write(
        crate_dir.join("Cargo.toml"),
        "[dependencies]\nserde = \"1\"\n\n[dev-dependencies]\ncrewmux-mux.workspace = true\n",
    )
    .unwrap();
    lint_mcp_state_path(dir.path()).unwrap();
}
