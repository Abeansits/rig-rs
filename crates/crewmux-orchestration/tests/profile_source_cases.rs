//! One case per `profile-source-resolver.test.ts` title.
#![expect(clippy::expect_used, reason = "fixture setup failures abort the case")]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crewmux_orchestration::context_packs::{SourceRoots, read_source};
use crewmux_spec::context::{parse_source_ref, source_kind_for_address};

struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "rig-src-{}-{}",
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

fn tree() -> (Tmp, PathBuf, PathBuf, PathBuf, PathBuf) {
    let tmp = Tmp::new();
    let pack = tmp.0.join("pack");
    let project = tmp.0.join("project-tree");
    let seat = tmp.0.join("seat-tree");
    let mission = tmp.0.join("mission-tree");
    for dir in [&pack, &project, &seat, &mission] {
        fs::create_dir_all(dir).expect("dir");
    }
    fs::write(pack.join("walk.md"), "## Welcome\nhello").expect("walk");
    fs::write(project.join("SPEC.md"), "# Project\nproject intent").expect("spec");
    fs::write(
        seat.join("RECAP.md"),
        "## Recent Decisions\nwe chose X because Y",
    )
    .expect("recap");
    fs::write(mission.join("NOTES.md"), "## Watch Items\nW-1").expect("notes");
    (tmp, pack, project, seat, mission)
}

#[test]
fn bare_refs_are_library_and_prefixes_name_tree_resolvers() {
    assert_eq!(
        parse_source_ref("walk.md").expect("lib").kind.as_str(),
        "library"
    );
    assert_eq!(parse_source_ref("walk.md").expect("lib").rel, "walk.md");
    assert_eq!(
        parse_source_ref("project:SPEC.md")
            .expect("p")
            .kind
            .as_str(),
        "project"
    );
    assert_eq!(
        parse_source_ref("project:SPEC.md").expect("p").rel,
        "SPEC.md"
    );
    assert_eq!(
        parse_source_ref("seat:RECAP.md").expect("s").kind.as_str(),
        "seat"
    );
    assert_eq!(
        parse_source_ref("seat:RECAP.md").expect("s").rel,
        "RECAP.md"
    );
    assert_eq!(
        parse_source_ref("mission:notes/NOTES.md")
            .expect("m")
            .kind
            .as_str(),
        "mission"
    );
    assert_eq!(
        parse_source_ref("mission:notes/NOTES.md").expect("m").rel,
        "notes/NOTES.md"
    );
}

#[test]
fn fails_loud_on_unknown_prefix_and_traversal() {
    for refer in [
        "library2:x.md",
        "seat:../LEARNED.md",
        "seat:/etc/passwd",
        "seat:",
    ] {
        assert!(parse_source_ref(refer).is_err(), "{refer}");
    }
}

#[test]
fn dispatches_library_refs_to_the_pack_and_tree_refs_to_configured_roots() {
    let (_tmp, pack, project, seat, mission) = tree();
    let roots = SourceRoots {
        project: Some(project),
        seat: Some(seat),
        mission: Some(mission),
    };
    let (walk, _) = read_source(&pack, &roots, "walk.md").expect("walk");
    let (spec, _) = read_source(&pack, &roots, "project:SPEC.md").expect("spec");
    let (recap, _) = read_source(&pack, &roots, "seat:RECAP.md").expect("recap");
    let (notes, _) = read_source(&pack, &roots, "mission:NOTES.md").expect("notes");
    assert!(walk.contains("hello"));
    assert!(spec.contains("project intent"));
    assert!(recap.contains("we chose X because Y"));
    assert!(notes.contains("W-1"));
}

#[test]
fn missing_file_names_the_source_kind_and_path() {
    let (_tmp, pack, _project, seat, mission) = tree();
    let roots = SourceRoots {
        seat: Some(seat),
        mission: Some(mission),
        ..SourceRoots::default()
    };
    let err = read_source(&pack, &roots, "seat:GONE.md").expect_err("gone");
    let message = err.message();
    assert!(message.contains("seat"));
    assert!(message.contains("GONE.md"));
}

#[test]
fn unconfigured_root_fails_loud_naming_the_missing_config() {
    let (_tmp, pack, _, _, _) = tree();
    let err =
        read_source(Path::new(&pack), &SourceRoots::default(), "seat:RECAP.md").expect_err("root");
    let message = err.message().to_ascii_lowercase();
    assert!(message.contains("seat") && message.contains("root"));
}

#[test]
fn source_kind_for_address_derives_the_label() {
    assert_eq!(
        source_kind_for_address("walk.md#welcome")
            .expect("l")
            .as_str(),
        "library"
    );
    assert_eq!(
        source_kind_for_address("project:SPEC.md")
            .expect("p")
            .as_str(),
        "project"
    );
    assert_eq!(
        source_kind_for_address("seat:RECAP.md#recent-decisions")
            .expect("s")
            .as_str(),
        "seat"
    );
    assert_eq!(
        source_kind_for_address("mission:NOTES.md")
            .expect("m")
            .as_str(),
        "mission"
    );
}
