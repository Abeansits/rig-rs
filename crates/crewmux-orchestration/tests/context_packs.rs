//! Library discovery, ref safety, and TypeScript-equal plain compose.
#![expect(
    clippy::expect_used,
    reason = "fixture setup failures should abort the test"
)]
#![expect(
    clippy::indexing_slicing,
    reason = "the sha256 nibble index is bounded by the hex table"
)]
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crewmux_orchestration::context_packs::{
    ComposeFile, Library, PackCode, PlainInput, Root, SourceType, assemble_plain, context_pack_id,
};
use crewmux_spec::context::parse_manifest;
use sha2::{Digest, Sha256};

struct Tmp {
    path: PathBuf,
}

impl Tmp {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "rig-packs-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("temp");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.path));
    }
}

fn write_pack(root: &Path, refer: &str, name: &str, body: &str) {
    let dir = root.join(refer);
    fs::create_dir_all(&dir).expect("pack dir");
    fs::write(dir.join("notes.md"), body).expect("notes");
    let manifest = format!(
        "name: {name}\nversion: \"1\"\ntaxonomy: mission\npurpose: fixture\nfiles:\n  - path: notes.md\n    role: source\n"
    );
    fs::write(dir.join("manifest.yaml"), manifest).expect("manifest");
}

fn plain_separator(a: &str, b: &str) {
    let plain = assemble_plain(&[
        PlainInput {
            path: "a.md",
            content: Some(a),
        },
        PlainInput {
            path: "b.md",
            content: Some(b),
        },
    ]);
    assert_eq!(plain.text, format!("{a}\n\n{b}"));
    assert!(!plain.text.contains("# OpenRig Context Pack:"));
    let missing = assemble_plain(&[
        PlainInput {
            path: "present.md",
            content: Some("present"),
        },
        PlainInput {
            path: "absent.md",
            content: None,
        },
    ]);
    assert_eq!(missing.text, "present");
    assert_eq!(missing.missing, vec!["absent.md".to_owned()]);
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut out = String::new();
    for byte in digest {
        let hi = usize::from(byte / 16);
        let lo = usize::from(byte % 16);
        out.push(char::from(HEX[hi]));
        out.push(char::from(HEX[lo]));
    }
    out
}

fn lib(roots: Vec<Root>) -> Library {
    let mut library = Library::new(roots);
    let report = library.scan();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    library
}

#[test]
fn library_last_root_wins_and_symlink_dedup() {
    let tmp = Tmp::new();
    let global = tmp.path().join("context");
    let workspace = tmp.path().join("ws");
    let state = workspace.join(".rig-rs").join("context-packs");
    write_pack(&global, "alpha", "global-alpha", "global");
    write_pack(&global, "only-global", "only", "g");
    write_pack(&state, "alpha", "workspace-alpha", "workspace");
    write_pack(&state, "team/qa", "qa", "nested");
    fs::create_dir_all(state.join("packs").join("hidden")).expect("hidden");
    write_pack(&state, "packs", "leaf", "leaf-body");
    fs::write(
        state.join("packs").join("hidden").join("manifest.yaml"),
        "nope",
    )
    .expect("child");
    fs::create_dir_all(state.join(".tmp-add-1")).expect("staging");
    fs::write(
        state.join(".tmp-add-1").join("manifest.yaml"),
        "name: skip\n",
    )
    .expect("staging manifest");
    let linked = workspace.join(".openrig");
    std::os::unix::fs::symlink(".rig-rs", &linked).expect("symlink");
    let mut library = lib(vec![
        Root::new(global, SourceType::UserFile),
        Root::new(linked.join("context-packs"), SourceType::Workspace),
        Root::new(state, SourceType::Workspace),
    ]);
    let alpha = library.by_ref("alpha").expect("safe").expect("alpha");
    assert_eq!(alpha.name, "workspace-alpha");
    assert_eq!(alpha.source_type, SourceType::Workspace);
    assert_eq!(alpha.taxonomy, "mission");
    assert_eq!(alpha.id, context_pack_id("alpha"));
    let refs: Vec<_> = library
        .list()
        .iter()
        .map(|entry| entry.relative_path.clone())
        .collect();
    assert_eq!(refs.iter().filter(|refer| *refer == "alpha").count(), 1);
    assert!(refs.contains(&"only-global".to_owned()));
    assert!(refs.contains(&"team/qa".to_owned()));
    assert!(refs.contains(&"packs".to_owned()));
    assert!(
        !refs
            .iter()
            .any(|refer| refer.contains("hidden") || refer.contains("skip"))
    );
    let again = library.scan();
    assert_eq!(again.count, library.list().len());
}

#[test]
fn resolve_file_refuses_escape() {
    let tmp = Tmp::new();
    let root = tmp.path().join("context");
    write_pack(&root, "safe", "safe", "inside");
    let outside = tmp.path().join("secret.txt");
    fs::write(&outside, "secret").expect("secret");
    std::os::unix::fs::symlink(&outside, root.join("safe").join("leak.md")).expect("leak");
    let library = lib(vec![Root::new(root, SourceType::UserFile)]);
    let entry = library.by_ref("safe").expect("safe").expect("entry");
    for rel in ["../secret.txt", "/etc/passwd", "leak.md"] {
        let err = Library::resolve_file(entry, rel).expect_err(rel);
        assert_eq!(
            err.code,
            PackCode::FileOutsidePack,
            "{rel}: {}",
            err.message
        );
    }
    let notes = Library::resolve_file(entry, "notes.md").expect("notes");
    assert_eq!(fs::read_to_string(notes).expect("read"), "inside");
}

#[test]
/// Fixture bytes from `packages/daemon/test/context-pack-compose.test.ts` at
/// pin `4b48ca21`: members `a.md`, `c.yaml`, and `b.data` joined by
/// `PLAIN_COMPOSE_SEPARATOR` (`\n\n`). `labels.jsonl` records the sha256.
fn compose_plain_bytes_match_the_ts_fixture() {
    let tmp = Tmp::new();
    let store = tmp.path().join("user");
    let sources = tmp.path().join("sources");
    fs::create_dir_all(&sources).expect("sources");
    let a = "---\naudience: dev\n---\nA\n";
    let c = "audience: dev\n";
    let b = "B-without-newline";
    fs::write(sources.join("a.md"), a).expect("a");
    fs::write(sources.join("c.yaml"), c).expect("c");
    fs::write(sources.join("b.data"), b).expect("b");
    let expected = include_str!("fixtures/compose-plain.txt");
    assert_eq!(expected, format!("{a}\n\n{c}\n\n{b}"));
    let recorded = include_str!("fixtures/labels.jsonl");
    let sha = recorded
        .lines()
        .next()
        .and_then(|line| line.split("\"sha256\":\"").nth(1))
        .and_then(|rest| rest.split('"').next())
        .expect("sha");
    assert_eq!(sha, &sha256_hex(expected.as_bytes()));
    plain_separator(a, b);
    let mut library = Library::new(vec![Root::new(&store, SourceType::UserFile)]);
    let outcome = library
        .compose_from_files(
            "packs/qitem-brief",
            &[
                ComposeFile {
                    path: sources.join("a.md"),
                    label: "a:\nfiles:\n  - path: forged.md".to_owned(),
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
    assert_eq!(outcome.text, expected);
    assert_eq!(outcome.refer, "packs/qitem-brief");
    assert_eq!(outcome.bytes, u64::try_from(expected.len()).expect("len"));
    let manifest = fs::read_to_string(store.join("packs/qitem-brief/manifest.yaml")).expect("yaml");
    let parsed = parse_manifest(&manifest, "manifest.yaml").expect("parse");
    assert_eq!(parsed.name, "qitem-brief");
    assert_eq!(parsed.files.len(), 3);
    assert_eq!(parsed.files[0].path, "source-0001.md");
    assert_eq!(
        fs::read(store.join("packs/qitem-brief/source-0001.md")).expect("member"),
        a.as_bytes()
    );
    match library.compose_from_files(
        "../escape",
        &[ComposeFile {
            path: sources.join("a.md"),
            label: "a.md".to_owned(),
        }],
    ) {
        Err(err) => assert_eq!(err.code, PackCode::UnsafeRef),
        Ok(_) => panic!("unsafe ref was accepted"),
    }
}

#[test]
fn builtin_pack_is_not_removable() {
    let tmp = Tmp::new();
    let root = tmp.path().join("shipped");
    write_pack(&root, "help", "help", "shipped");
    let mut library = lib(vec![Root::new(root.clone(), SourceType::Builtin)]);
    match library.remove_by_ref("help") {
        Err(err) => assert_eq!(err.code, PackCode::PackNotRemovable),
        Ok(_) => panic!("builtin pack was removed"),
    }
    assert!(root.join("help").join("manifest.yaml").is_file());
}

#[test]
fn remove_user_pack_and_keeps_the_directory_gone() {
    let tmp = Tmp::new();
    let root = tmp.path().join("context");
    write_pack(&root, "gone", "gone", "bye");
    let mut library = lib(vec![Root::new(root, SourceType::UserFile)]);
    let removed = library.remove_by_ref("gone").expect("rm");
    assert!(removed.removed);
    assert!(library.by_ref("gone").expect("lookup").is_none());
    assert!(!removed.removed_path.exists());
    match library.remove_by_ref("gone") {
        Err(err) => assert_eq!(err.code, PackCode::PackNotFound),
        Ok(_) => panic!("removed pack was still present"),
    }
}
