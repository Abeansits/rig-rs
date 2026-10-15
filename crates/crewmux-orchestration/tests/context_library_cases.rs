//! One case per `context-pack-library-service.test.ts` title.
#![expect(clippy::expect_used, reason = "fixture setup failures abort the case")]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crewmux_orchestration::context_packs::{
    Library, PackCode, Root, SourceType, context_pack_id, estimate_tokens_from_bytes,
};

struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "rig-lib-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("temp");
        Self(path)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.0));
    }
}

fn write_pack(root: &Path, name: &str, manifest: &str, files: &[(&str, &str)]) {
    let dir = root.join(name);
    fs::create_dir_all(&dir).expect("pack");
    fs::write(dir.join("manifest.yaml"), manifest).expect("manifest");
    for (path, body) in files {
        let file = dir.join(path);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent).expect("parent");
        }
        fs::write(file, body).expect("file");
    }
}

#[test]
fn scans_a_pack_and_emits_a_normalized_entry() {
    let tmp = Tmp::new();
    let root = tmp.0.join("user");
    write_pack(
        &root,
        "smoke",
        "name: smoke\nversion: 1\ntaxonomy: world\npurpose: Smoke pack\nfiles:\n  - path: notes.md\n    role: notes\n    summary: Smoke notes\n",
        &[("notes.md", "# Smoke\n\nHello world.")],
    );
    let mut library = Library::new(vec![Root::new(&root, SourceType::UserFile)]);
    let report = library.scan();
    assert_eq!(report.count, 1);
    assert_eq!(report.errors, []);
    let entry = library.list().into_iter().next().expect("entry");
    assert_eq!(entry.id, context_pack_id("smoke"));
    assert_eq!(entry.name, "smoke");
    assert_eq!(entry.to_json()["kind"], "context-pack");
    assert_eq!(entry.purpose.as_deref(), Some("Smoke pack"));
    assert_eq!(entry.files.len(), 1);
    assert!(entry.files[0].bytes.unwrap_or(0) > 0);
    assert!(entry.files[0].estimated_tokens.unwrap_or(0) > 0);
    assert_eq!(
        entry.derived_estimated_tokens,
        entry.files[0].estimated_tokens.unwrap_or(0)
    );
}

#[test]
fn surfaces_missing_files_with_null_bytes() {
    let tmp = Tmp::new();
    let root = tmp.0.join("user");
    write_pack(
        &root,
        "missing",
        "name: missing\nversion: 1\ntaxonomy: world\nfiles:\n  - path: present.md\n    role: r\n  - path: absent.md\n    role: r\n",
        &[("present.md", "data")],
    );
    let mut library = Library::new(vec![Root::new(&root, SourceType::UserFile)]);
    drop(library.scan());
    let entry = library.by_ref("missing").expect("ref").expect("entry");
    let present = entry
        .files
        .iter()
        .find(|file| file.path == "present.md")
        .expect("present");
    let absent = entry
        .files
        .iter()
        .find(|file| file.path == "absent.md")
        .expect("absent");
    assert!(present.bytes.unwrap_or(0) > 0);
    assert_eq!(absent.bytes, None);
    assert_eq!(absent.estimated_tokens, None);
}

#[test]
fn workspace_root_wins_on_collision() {
    let tmp = Tmp::new();
    let user = tmp.0.join("user");
    let workspace = tmp.0.join("workspace");
    let manifest =
        "name: collision\nversion: 1\ntaxonomy: world\nfiles:\n  - path: notes.md\n    role: r\n";
    write_pack(
        &user,
        "collision",
        manifest,
        &[("notes.md", "user content")],
    );
    write_pack(
        &workspace,
        "collision",
        manifest,
        &[("notes.md", "workspace content")],
    );
    let mut library = Library::new(vec![
        Root::new(&user, SourceType::UserFile),
        Root::new(&workspace, SourceType::Workspace),
    ]);
    drop(library.scan());
    let entry = library.by_ref("collision").expect("ref").expect("entry");
    assert_eq!(entry.source_type, SourceType::Workspace);
    assert!(
        entry
            .source_path
            .display()
            .to_string()
            .contains("/workspace/")
    );
}

#[test]
fn distinct_refs_sharing_name_and_version_get_distinct_ids() {
    let tmp = Tmp::new();
    let root = tmp.0.join("user");
    let manifest =
        "name: dup\nversion: 1\ntaxonomy: world\nfiles:\n  - path: notes.md\n    role: r\n";
    write_pack(&root, "packs/a", manifest, &[("notes.md", "A")]);
    write_pack(&root, "packs/b", manifest, &[("notes.md", "B")]);
    let mut library = Library::new(vec![Root::new(&root, SourceType::UserFile)]);
    drop(library.scan());
    let a = library.by_ref("packs/a").expect("ref").expect("a");
    let b = library.by_ref("packs/b").expect("ref").expect("b");
    assert_eq!(a.id, "context-pack:packs/a");
    assert_eq!(b.id, "context-pack:packs/b");
    assert_ne!(a.id, b.id);
}

#[test]
fn captures_parse_errors_instead_of_throwing() {
    let tmp = Tmp::new();
    let root = tmp.0.join("user");
    write_pack(&root, "broken", "{not valid yaml", &[("notes.md", "x")]);
    let mut library = Library::new(vec![Root::new(&root, SourceType::UserFile)]);
    let report = library.scan();
    assert_eq!(report.count, 0);
    assert_eq!(report.errors.len(), 1);
    assert!(report.errors[0].error.contains("manifest_parse_error"));
}

#[test]
fn ignores_directories_without_manifest_yaml() {
    let tmp = Tmp::new();
    let root = tmp.0.join("user");
    fs::create_dir_all(root.join("not-a-pack")).expect("dir");
    let mut library = Library::new(vec![Root::new(&root, SourceType::UserFile)]);
    assert_eq!(library.scan().count, 0);
}

#[test]
fn rescan_reflects_filesystem_edits() {
    let tmp = Tmp::new();
    let root = tmp.0.join("user");
    write_pack(
        &root,
        "evolve",
        "name: evolve\nversion: 1\ntaxonomy: world\nfiles:\n  - path: a.md\n    role: r\n",
        &[("a.md", "initial")],
    );
    let mut library = Library::new(vec![Root::new(&root, SourceType::UserFile)]);
    drop(library.scan());
    assert_eq!(library.list().len(), 1);
    fs::write(
        root.join("evolve/manifest.yaml"),
        "name: evolve\nversion: 2\ntaxonomy: world\nfiles:\n  - path: a.md\n    role: r\n",
    )
    .expect("edit");
    drop(library.scan());
    assert_eq!(library.list()[0].version, "2");
}

#[test]
fn resolve_file_within_pack_rejects_traversal() {
    let tmp = Tmp::new();
    let root = tmp.0.join("user");
    write_pack(
        &root,
        "guard",
        "name: guard\nversion: 1\ntaxonomy: world\nfiles:\n  - path: notes.md\n    role: r\n",
        &[("notes.md", "x")],
    );
    let mut library = Library::new(vec![Root::new(&root, SourceType::UserFile)]);
    drop(library.scan());
    let entry = library
        .by_ref("guard")
        .expect("ref")
        .expect("entry")
        .clone();
    for rel in ["../etc/passwd", "/abs"] {
        let err = Library::resolve_file(&entry, rel).expect_err(rel);
        assert_eq!(err.code, PackCode::FileOutsidePack);
        assert!(err.message.contains("inside the pack"));
    }
}

#[test]
fn builds_the_opaque_context_pack_id() {
    assert_eq!(
        context_pack_id("packs/compaction-restore"),
        "context-pack:packs/compaction-restore"
    );
    assert_eq!(context_pack_id("smoke"), "context-pack:smoke");
}

#[test]
fn uses_the_chars_over_four_heuristic() {
    assert_eq!(estimate_tokens_from_bytes(0), 0);
    assert_eq!(estimate_tokens_from_bytes(4), 1);
    assert_eq!(estimate_tokens_from_bytes(7), 2);
    assert_eq!(estimate_tokens_from_bytes(100), 25);
}

#[test]
fn indexes_neither_a_colon_bearing_nor_an_overlong_version() {
    let tmp = Tmp::new();
    let root = tmp.0.join("user");
    write_pack(
        &root,
        "colonver",
        "name: colonver\nversion: '1:0:0'\nfiles: []\n",
        &[],
    );
    write_pack(
        &root,
        "longver",
        &format!("name: longver\nversion: '{}'\nfiles: []\n", "a".repeat(300)),
        &[],
    );
    let mut library = Library::new(vec![Root::new(&root, SourceType::UserFile)]);
    let report = library.scan();
    assert_eq!(report.count, 0);
    assert_eq!(report.errors.len(), 2);
    assert!(
        report
            .errors
            .iter()
            .all(|err| err.error.contains("version"))
    );
    assert!(library.by_ref("colonver").expect("ref").is_none());
    assert!(library.by_ref("longver").expect("ref").is_none());
}
