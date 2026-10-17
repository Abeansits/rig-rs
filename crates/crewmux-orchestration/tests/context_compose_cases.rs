//! One case per `context-pack-compose.test.ts` title.
#![expect(clippy::expect_used, reason = "fixture setup failures abort the case")]

use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crewmux_orchestration::context_packs::{
    ComposeFile, Library, PackCode, PlainInput, Root, SourceType, assemble_plain,
    set_compose_member_writer_for_tests,
};
use crewmux_spec::context::parse_manifest;
use sha2::{Digest, Sha256};

struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "rig-compose-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("temp");
        Self(path)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        set_compose_member_writer_for_tests(None);
        drop(fs::remove_dir_all(&self.0));
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut out, byte| {
            push_hex(&mut out, *byte);
            out
        })
}

fn push_hex(out: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let hi = usize::from(byte / 16);
    let lo = usize::from(byte % 16);
    if let (Some(hi), Some(lo)) = (HEX.get(hi), HEX.get(lo)) {
        out.push(char::from(*hi));
        out.push(char::from(*lo));
    }
}

fn assert_member(target: &Path, name: &str, bytes: &str) {
    assert_eq!(fs::read(target.join(name)).expect(name), bytes.as_bytes());
}

fn recorded_sha(key: &str) -> &'static str {
    let recorded = include_str!("fixtures/labels.jsonl");
    let line = recorded
        .lines()
        .find(|line| line.contains(key))
        .expect("row");
    line.split("\"sha256\":\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("sha")
}

fn write_pack(root: &Path, refer: &str, name: &str) {
    let dir = root.join(refer);
    fs::create_dir_all(&dir).expect("pack");
    fs::write(
        dir.join("manifest.yaml"),
        format!("name: {name}\nversion: 1\ntaxonomy: mission\nfiles:\n  - path: notes.md\n    role: source\n"),
    )
    .expect("manifest");
    fs::write(dir.join("notes.md"), "existing bytes").expect("notes");
}

#[test]
fn preserves_eof_newline_bytes() {
    let rows = [("A", "B"), ("A\n", "B"), ("A", "B\n"), ("A\n", "B\n")];
    let recorded = include_str!("fixtures/labels.jsonl");
    for (index, (left, right)) in rows.iter().enumerate() {
        let assembled = assemble_plain(&[
            PlainInput {
                path: "a.md",
                content: Some(left),
            },
            PlainInput {
                path: "b.md",
                content: Some(right),
            },
        ]);
        let expected = format!("{left}\n\n{right}");
        assert_eq!(assembled.text, expected);
        assert!(!assembled.text.contains("# OpenRig Context Pack:"));
        assert!(!assembled.text.contains("## File:"));
        assert_eq!(assembled.bytes, u64::try_from(expected.len()).expect("len"));
        let key = format!("compose-newline-{index}");
        let line = recorded
            .lines()
            .find(|line| line.contains(&key))
            .expect("row");
        let sha = line
            .split("\"sha256\":\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .expect("sha");
        assert_eq!(sha, sha256_hex(expected.as_bytes()));
    }
}

#[test]
fn surfaces_missing_members_instead_of_fabricating_content() {
    let assembled = assemble_plain(&[
        PlainInput {
            path: "present.md",
            content: Some("present"),
        },
        PlainInput {
            path: "absent.md",
            content: None,
        },
    ]);
    assert_eq!(assembled.text, "present");
    assert_eq!(assembled.missing, vec!["absent.md".to_owned()]);
}

#[test]
fn writes_one_byte_identical_member_per_source() {
    let tmp = Tmp::new();
    let store = tmp.0.join("user-store");
    let sources = tmp.0.join("sources");
    fs::create_dir_all(&sources).expect("sources");
    let a = "---\naudience: dev\n---\nA\n";
    let c = "audience: dev\n";
    let b = "B-without-newline";
    fs::write(sources.join("a.md"), a).expect("a");
    fs::write(sources.join("c.yaml"), c).expect("c");
    fs::write(sources.join("b.data"), b).expect("b");
    let label = "a:\nfiles:\n  - path: forged.md";
    let mut library = Library::new(vec![Root::new(&store, SourceType::UserFile)]);
    let outcome = library
        .compose_from_files(
            "packs/qitem-brief",
            &[
                ComposeFile {
                    path: sources.join("a.md"),
                    label: label.to_owned(),
                },
                ComposeFile {
                    path: sources.join("c.yaml"),
                    label: "c.yaml".to_owned(),
                },
                ComposeFile {
                    path: sources.join("b.data"),
                    label: "b.data".to_owned(),
                },
            ],
        )
        .expect("compose");
    let target = store.join("packs/qitem-brief");
    let manifest = parse_manifest(
        &fs::read_to_string(target.join("manifest.yaml")).expect("yaml"),
        "manifest.yaml",
    )
    .expect("manifest");
    assert_eq!(manifest.name, "qitem-brief");
    assert_eq!(manifest.version, "1");
    assert_eq!(manifest.purpose(), Some("Composed from 3 ordered files"));
    assert_eq!(manifest.taxonomy.as_str(), "mission");
    assert_eq!(manifest.files[0].path, "source-0001.md");
    assert_eq!(manifest.files[0].summary.as_deref(), Some(label));
    assert_eq!(manifest.files[1].path, "source-0002.yaml");
    assert_eq!(manifest.files[2].path, "source-0003.txt");
    assert_member(&target, "source-0001.md", a);
    assert_member(&target, "source-0002.yaml", c);
    assert_member(&target, "source-0003.txt", b);
    let expected = format!("{a}\n\n{c}\n\n{b}");
    assert_eq!(outcome.text, expected);
    assert_eq!(outcome.refer, "packs/qitem-brief");
    assert_eq!(outcome.entry.relative_path, "packs/qitem-brief");
    assert_eq!(
        library
            .list()
            .iter()
            .map(|entry| entry.relative_path.as_str())
            .collect::<Vec<_>>(),
        ["packs/qitem-brief"]
    );
    assert_eq!(
        library
            .by_ref("packs/qitem-brief")
            .expect("ref")
            .expect("entry")
            .source_path,
        target
    );
    assert_eq!(
        recorded_sha("compose-plain.txt"),
        sha256_hex(expected.as_bytes())
    );
}

#[test]
fn rejects_unsafe_refs_before_creating_the_store_root() {
    let tmp = Tmp::new();
    let store = tmp.0.join("user-store");
    let source = tmp.0.join("a.md");
    fs::write(&source, "A").expect("a");
    let mut library = Library::new(vec![Root::new(&store, SourceType::UserFile)]);
    let err = library
        .compose_from_files(
            "../escape",
            &[ComposeFile {
                path: source,
                label: "a.md".to_owned(),
            }],
        )
        .expect_err("unsafe");
    assert_eq!(err.code, PackCode::UnsafeRef);
    assert!(!store.exists());
}

#[test]
fn surfaces_missing_sources_and_performs_no_store_mutation() {
    let tmp = Tmp::new();
    let store = tmp.0.join("user-store");
    let missing = tmp.0.join("absent.md");
    let mut library = Library::new(vec![Root::new(&store, SourceType::UserFile)]);
    let err = library
        .compose_from_files(
            "packs/missing",
            &[ComposeFile {
                path: missing.clone(),
                label: "absent.md".to_owned(),
            }],
        )
        .expect_err("missing");
    assert_eq!(err.code, PackCode::MissingFiles);
    assert_eq!(
        err.details["missingFiles"][0],
        missing.display().to_string()
    );
    assert!(!store.exists());
}

#[test]
fn empty_source_list_is_missing_files_before_store_inspection() {
    let tmp = Tmp::new();
    let store = tmp.0.join("user-store");
    let mut library = Library::new(vec![Root::new(&store, SourceType::UserFile)]);
    let err = library
        .compose_from_files("packs/empty", &[])
        .expect_err("empty");
    assert_eq!(err.code, PackCode::MissingFiles);
    assert_eq!(err.details["missingFiles"], serde_json::json!([]));
    assert!(!store.exists());
}

#[test]
fn unreadable_non_file_sources_perform_no_store_mutation() {
    let tmp = Tmp::new();
    let store = tmp.0.join("user-store");
    let dir = tmp.0.join("directory.md");
    fs::create_dir(&dir).expect("dir");
    let mut library = Library::new(vec![Root::new(&store, SourceType::UserFile)]);
    let err = library
        .compose_from_files(
            "packs/unreadable",
            &[ComposeFile {
                path: dir,
                label: "directory.md".to_owned(),
            }],
        )
        .expect_err("dir");
    assert_eq!(err.code, PackCode::FileReadFailed);
    assert!(!store.exists());
}

#[test]
fn rejects_an_exact_ref_conflict_in_any_discovery_root() {
    let tmp = Tmp::new();
    let store = tmp.0.join("user-store");
    let workspace = tmp.0.join("workspace-store");
    write_pack(&workspace, "packs/taken", "workspace-pack");
    let source = tmp.0.join("a.md");
    fs::write(&source, "A").expect("a");
    let mut library = Library::new(vec![
        Root::new(&store, SourceType::UserFile),
        Root::new(&workspace, SourceType::Workspace),
    ]);
    let err = library
        .compose_from_files(
            "packs/taken",
            &[ComposeFile {
                path: source,
                label: "a.md".to_owned(),
            }],
        )
        .expect_err("exists");
    assert_eq!(err.code, PackCode::PackExists);
    assert!(!store.join("packs/taken").exists());
    assert_eq!(
        library
            .by_ref("packs/taken")
            .expect("ref")
            .expect("entry")
            .name,
        "workspace-pack"
    );
}

#[test]
fn source_preflight_precedes_an_existing_ref_conflict() {
    let tmp = Tmp::new();
    let store = tmp.0.join("user-store");
    let workspace = tmp.0.join("workspace-store");
    write_pack(&workspace, "packs/taken", "workspace-pack");
    let before = fs::read(workspace.join("packs/taken/manifest.yaml")).expect("before");
    let missing = tmp.0.join("absent.md");
    let mut library = Library::new(vec![
        Root::new(&store, SourceType::UserFile),
        Root::new(&workspace, SourceType::Workspace),
    ]);
    let err = library
        .compose_from_files(
            "packs/taken",
            &[ComposeFile {
                path: missing.clone(),
                label: "absent.md".to_owned(),
            }],
        )
        .expect_err("missing");
    assert_eq!(err.code, PackCode::MissingFiles);
    assert_eq!(
        err.details["missingFiles"][0],
        missing.display().to_string()
    );
    assert!(!store.join("packs/taken").exists());
    assert_eq!(
        fs::read(workspace.join("packs/taken/manifest.yaml")).expect("after"),
        before
    );
}

#[test]
fn rejects_a_manifestless_physical_user_target() {
    let tmp = Tmp::new();
    let store = tmp.0.join("user-store");
    let target = store.join("packs/taken");
    fs::create_dir_all(&target).expect("target");
    fs::write(target.join("sentinel"), "keep-me").expect("sentinel");
    let source = tmp.0.join("a.md");
    fs::write(&source, "A").expect("a");
    let mut library = Library::new(vec![Root::new(&store, SourceType::UserFile)]);
    let err = library
        .compose_from_files(
            "packs/taken",
            &[ComposeFile {
                path: source,
                label: "a.md".to_owned(),
            }],
        )
        .expect_err("exists");
    assert_eq!(err.code, PackCode::PackExists);
    assert_eq!(
        fs::read_to_string(target.join("sentinel")).expect("sentinel"),
        "keep-me"
    );
    assert!(!target.join("manifest.yaml").exists());
}

#[test]
fn rejects_a_ref_below_a_manifest_bearing_leaf() {
    let tmp = Tmp::new();
    let store = tmp.0.join("user-store");
    write_pack(&store, "packs", "leaf-pack");
    let source = tmp.0.join("a.md");
    fs::write(&source, "A").expect("a");
    let mut library = Library::new(vec![Root::new(&store, SourceType::UserFile)]);
    let err = library
        .compose_from_files(
            "packs/hidden-child",
            &[ComposeFile {
                path: source,
                label: "a.md".to_owned(),
            }],
        )
        .expect_err("below");
    assert_eq!(err.code, PackCode::PackRefBelowPack);
    assert!(!store.join("packs/hidden-child").exists());
}

#[test]
fn rejects_symlink_and_non_directory_namespace_segments() {
    let tmp = Tmp::new();
    let store = tmp.0.join("user-store");
    fs::create_dir_all(&store).expect("store");
    let outside = tmp.0.join("outside");
    fs::create_dir(&outside).expect("outside");
    std::os::unix::fs::symlink(&outside, store.join("linked")).expect("link");
    let source = tmp.0.join("a.md");
    fs::write(&source, "A").expect("a");
    let mut library = Library::new(vec![Root::new(&store, SourceType::UserFile)]);
    let err = library
        .compose_from_files(
            "linked/escape",
            &[ComposeFile {
                path: source.clone(),
                label: "a.md".to_owned(),
            }],
        )
        .expect_err("link");
    assert_eq!(err.code, PackCode::UnsafeRefNamespace);
    assert!(!outside.join("escape").exists());
    fs::write(store.join("blocked"), "not-a-directory").expect("file");
    let err = library
        .compose_from_files(
            "blocked/child",
            &[ComposeFile {
                path: source,
                label: "a.md".to_owned(),
            }],
        )
        .expect_err("file");
    assert_eq!(err.code, PackCode::UnsafeRefNamespace);
    assert_eq!(
        fs::read_to_string(store.join("blocked")).expect("blocked"),
        "not-a-directory"
    );
}

thread_local! {
    static WRITES: Cell<u32> = const { Cell::new(0) };
}

fn failing_member(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let count = WRITES.with(|cell| {
        let next = cell.get().saturating_add(1);
        cell.set(next);
        next
    });
    if count == 2 {
        return Err(std::io::Error::other("injected write failure"));
    }
    fs::write(path, bytes)
}

#[test]
fn cleans_a_newly_created_target_when_a_member_write_fails() {
    let tmp = Tmp::new();
    let store = tmp.0.join("user-store");
    let sources = tmp.0.join("sources");
    fs::create_dir_all(&sources).expect("sources");
    fs::write(sources.join("a.md"), "A").expect("a");
    fs::write(sources.join("b.md"), "B").expect("b");
    WRITES.with(|cell| cell.set(0));
    set_compose_member_writer_for_tests(Some(failing_member));
    let mut library = Library::new(vec![Root::new(&store, SourceType::UserFile)]);
    let err = library
        .compose_from_files(
            "packs/write-fails",
            &[
                ComposeFile {
                    path: sources.join("a.md"),
                    label: "a.md".to_owned(),
                },
                ComposeFile {
                    path: sources.join("b.md"),
                    label: "b.md".to_owned(),
                },
            ],
        )
        .expect_err("write");
    set_compose_member_writer_for_tests(None);
    assert_eq!(err.code, PackCode::PackWriteFailed);
    assert!(!store.join("packs/write-fails").exists());
    assert!(library.by_ref("packs/write-fails").expect("ref").is_none());
}
