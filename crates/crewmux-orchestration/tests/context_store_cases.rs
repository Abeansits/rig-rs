//! One case per `context-pack-store-recursion.test.ts` title.
#![expect(clippy::expect_used, reason = "fixture setup failures abort the case")]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crewmux_orchestration::context_packs::{Library, PackCode, Root, SourceType};

struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "rig-store-{}-{}",
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

fn write_pack(root: &Path, refer: &str, name: &str, version: &str) {
    let dir = root.join(refer);
    fs::create_dir_all(&dir).expect("pack");
    fs::write(
        dir.join("manifest.yaml"),
        format!("name: {name}\nversion: {version}\ntaxonomy: world\npurpose: test pack\nfiles:\n  - path: notes.md\n    role: notes\n"),
    )
    .expect("manifest");
    fs::write(dir.join("notes.md"), format!("# {name}\n")).expect("notes");
}

fn user(root: &Path) -> Library {
    Library::new(vec![Root::new(root, SourceType::UserFile)])
}

#[test]
fn discovers_the_specs_own_example_refs() {
    let tmp = Tmp::new();
    let root = tmp.0.join("store");
    write_pack(&root, "packs/compaction-restore", "compaction-restore", "1");
    write_pack(&root, "as-built/queue-internals", "queue-internals", "1");
    let mut library = user(&root);
    let report = library.scan();
    assert_eq!(report.errors, []);
    assert_eq!(report.count, 2);
    let mut refs: Vec<_> = library
        .list()
        .iter()
        .map(|entry| entry.relative_path.clone())
        .collect();
    refs.sort();
    assert_eq!(
        refs,
        ["as-built/queue-internals", "packs/compaction-restore"]
    );
}

#[test]
fn one_level_packs_keep_their_dir_name_and_deeper_nesting_works() {
    let tmp = Tmp::new();
    let root = tmp.0.join("store");
    write_pack(&root, "flat", "flat", "1");
    write_pack(&root, "a/b/c", "deep", "1");
    let mut library = user(&root);
    assert_eq!(library.scan().count, 2);
    let mut refs: Vec<_> = library
        .list()
        .iter()
        .map(|entry| entry.relative_path.clone())
        .collect();
    refs.sort();
    assert_eq!(refs, ["a/b/c", "flat"]);
}

#[test]
fn overlapping_startup_roots_assign_one_identity_to_a_system_world_pack() {
    let tmp = Tmp::new();
    let root = tmp.0.join("store");
    write_pack(&root, "system/baseline", "system-baseline", "1");
    let mut library = Library::new(vec![
        Root::new(&root, SourceType::UserFile),
        Root::new(root.join("system"), SourceType::UserFile),
    ]);
    let report = library.scan();
    assert_eq!(report.count, 1);
    assert_eq!(report.errors, []);
    assert_eq!(library.list()[0].relative_path, "system/baseline");
    assert!(library.by_ref("baseline").expect("ref").is_none());
}

#[test]
fn same_name_top_level_and_system_world_packs_stay_distinct() {
    let tmp = Tmp::new();
    let root = tmp.0.join("store");
    write_pack(&root, "baseline", "operator-baseline", "1");
    write_pack(&root, "system/baseline", "system-baseline", "1");
    let mut library = Library::new(vec![
        Root::new(&root, SourceType::UserFile),
        Root::new(root.join("system"), SourceType::UserFile),
    ]);
    assert_eq!(library.scan().count, 2);
    assert_eq!(
        library
            .by_ref("baseline")
            .expect("ref")
            .expect("top")
            .source_path,
        root.join("baseline")
    );
    assert_eq!(
        library
            .by_ref("system/baseline")
            .expect("ref")
            .expect("sys")
            .source_path,
        root.join("system/baseline")
    );
}

#[test]
fn nested_alias_root_keeps_physical_pack_custody() {
    for collision in [false, true] {
        let tmp = Tmp::new();
        let root = tmp.0.join("store");
        write_pack(&root, "system/baseline", "system-baseline", "1");
        if collision {
            write_pack(&root, "baseline", "operator-baseline", "1");
        }
        let alias = tmp.0.join("system-alias");
        std::os::unix::fs::symlink(root.join("system"), &alias).expect("alias");
        let mut library = Library::new(vec![
            Root::new(&root, SourceType::UserFile),
            Root::new(&alias, SourceType::Workspace),
        ]);
        let report = library.scan();
        assert_eq!(report.errors, []);
        assert_eq!(report.count, if collision { 2 } else { 1 });
        assert_eq!(
            library
                .by_ref("system/baseline")
                .expect("ref")
                .expect("sys")
                .source_path,
            root.join("system/baseline")
        );
        if collision {
            assert_eq!(
                library.by_ref("baseline").expect("ref").expect("top").name,
                "operator-baseline"
            );
            let removed = library.remove_by_ref("baseline").expect("remove");
            assert_eq!(removed.removed_path, root.join("baseline"));
            assert!(root.join("system/baseline/manifest.yaml").is_file());
            assert_eq!(
                library
                    .by_ref("system/baseline")
                    .expect("ref")
                    .expect("sys")
                    .name,
                "system-baseline"
            );
            assert!(library.by_ref("baseline").expect("ref").is_none());
        } else {
            assert!(library.by_ref("baseline").expect("ref").is_none());
        }
    }
}

#[test]
fn packs_are_leaves() {
    let tmp = Tmp::new();
    let root = tmp.0.join("store");
    write_pack(&root, "outer", "outer", "1");
    write_pack(&root, "outer/inner", "inner", "1");
    let mut library = user(&root);
    assert_eq!(library.scan().count, 1);
    assert_eq!(library.list()[0].relative_path, "outer");
}

#[test]
fn unsafe_on_disk_ref_is_skipped_and_fail_visible() {
    let tmp = Tmp::new();
    let root = tmp.0.join("store");
    write_pack(&root, "bad name", "badpack", "1");
    write_pack(&root, "good", "goodpack", "1");
    let mut library = user(&root);
    let report = library.scan();
    assert_eq!(report.count, 1);
    assert_eq!(library.list()[0].relative_path, "good");
    assert_eq!(report.errors.len(), 1);
    assert!(report.errors[0].source.contains("bad name"));
    assert!(report.errors[0].error.contains("unsafe pack ref"));
    assert!(report.errors[0].error.contains("'/'-separated segments"));
}

#[test]
fn get_by_ref_returns_the_entry_or_null() {
    let tmp = Tmp::new();
    let root = tmp.0.join("store");
    write_pack(&root, "packs/compaction-restore", "compaction-restore", "1");
    let mut library = user(&root);
    drop(library.scan());
    let entry = library
        .by_ref("packs/compaction-restore")
        .expect("ref")
        .expect("entry");
    assert_eq!(entry.name, "compaction-restore");
    assert_eq!(entry.relative_path, "packs/compaction-restore");
    assert!(library.by_ref("packs/absent").expect("ref").is_none());
}

#[test]
fn resolve_boundary_rejects_unsafe_refs() {
    let refs = [
        "../escape",
        "a/../b",
        "packs/./x",
        "/abs",
        "a//b",
        "a/",
        "bad name",
        "a:b",
        "",
        ".hidden/x",
    ];
    let tmp = Tmp::new();
    let root = tmp.0.join("store");
    write_pack(&root, "good", "goodpack", "1");
    let mut library = user(&root);
    drop(library.scan());
    for refer in refs {
        let err = library.by_ref(refer).expect_err(refer);
        assert_eq!(err.code, PackCode::UnsafeRef);
        assert!(err.message.contains("unsafe pack ref"));
    }
}

#[test]
fn distinct_refs_with_identical_manifests_stay_independent() {
    let tmp = Tmp::new();
    let root = tmp.0.join("store");
    write_pack(&root, "packs/a", "same", "1");
    write_pack(&root, "packs/b", "same", "1");
    let mut library = user(&root);
    let report = library.scan();
    assert_eq!(report.errors, []);
    assert_eq!(report.count, 2);
    assert!(
        library
            .by_ref("packs/a")
            .expect("ref")
            .expect("a")
            .source_path
            .ends_with("packs/a")
    );
    assert!(
        library
            .by_ref("packs/b")
            .expect("ref")
            .expect("b")
            .source_path
            .ends_with("packs/b")
    );
}

#[test]
fn same_ref_across_roots_is_last_root_wins_everywhere() {
    let tmp = Tmp::new();
    let root = tmp.0.join("store");
    let other = tmp.0.join("storeB2");
    write_pack(&root, "packs/dup", "first", "1");
    write_pack(&other, "packs/dup", "second", "2");
    let mut library = Library::new(vec![
        Root::new(&root, SourceType::UserFile),
        Root::new(&other, SourceType::UserFile),
    ]);
    let report = library.scan();
    assert_eq!(report.count, 1);
    assert_eq!(library.list().len(), 1);
    let entry = library.by_ref("packs/dup").expect("ref").expect("entry");
    assert_eq!(entry.name, "second");
    assert_eq!(entry.version, "2");
}

#[test]
fn last_root_wins_when_two_roots_serve_the_same_ref() {
    let tmp = Tmp::new();
    let root = tmp.0.join("store");
    let other = tmp.0.join("storeB");
    write_pack(&root, "alpha", "first", "1");
    write_pack(&other, "alpha", "second", "2");
    let mut library = Library::new(vec![
        Root::new(&root, SourceType::UserFile),
        Root::new(&other, SourceType::Workspace),
    ]);
    drop(library.scan());
    assert_eq!(
        library
            .by_ref("alpha")
            .expect("ref")
            .expect("entry")
            .version,
        "2"
    );
}

#[test]
fn same_ref_override_keeps_distinct_ids() {
    let tmp = Tmp::new();
    let root = tmp.0.join("store");
    let other = tmp.0.join("storeB3");
    write_pack(&root, "packs/a", "same", "1");
    write_pack(&root, "packs/b", "same", "1");
    write_pack(&other, "packs/a", "same", "1");
    let mut library = Library::new(vec![
        Root::new(&root, SourceType::Builtin),
        Root::new(&other, SourceType::UserFile),
    ]);
    assert_eq!(library.scan().count, 2);
    let a = library.by_ref("packs/a").expect("ref").expect("a");
    let b = library.by_ref("packs/b").expect("ref").expect("b");
    assert_eq!(a.source_type, SourceType::UserFile);
    assert_eq!(b.source_type, SourceType::Builtin);
    assert_eq!(a.id, "context-pack:packs/a");
    assert_eq!(b.id, "context-pack:packs/b");
    assert_ne!(a.id, b.id);
}

#[test]
fn colon_id_addressing_is_removed() {
    let tmp = Tmp::new();
    let root = tmp.0.join("store");
    write_pack(&root, "packs/compaction-restore", "compaction-restore", "3");
    let mut library = user(&root);
    drop(library.scan());
    let entry = library
        .by_ref("packs/compaction-restore")
        .expect("ref")
        .expect("entry");
    assert_eq!(entry.id, "context-pack:packs/compaction-restore");
    assert!(library.by_ref("compaction-restore:3").is_err());
}

#[test]
fn symlinked_directories_are_not_traversed() {
    let tmp = Tmp::new();
    let root = tmp.0.join("store");
    write_pack(&root, "real/target", "target", "1");
    std::os::unix::fs::symlink(root.join("real"), root.join("alias")).expect("alias");
    let mut library = user(&root);
    let report = library.scan();
    assert_eq!(report.count, 1);
    assert_eq!(library.list()[0].relative_path, "real/target");
}
