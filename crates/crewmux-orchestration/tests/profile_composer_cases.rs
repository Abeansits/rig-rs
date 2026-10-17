//! One case per `profile-composer.test.ts` title.
#![expect(clippy::expect_used, reason = "fixture setup failures abort the case")]

use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

use crewmux_orchestration::context_packs::{
    ComposeError, ComposedProfile, ReadFail, SourceRoots, compose_profile, read_source,
};
use crewmux_spec::context::{Atom, AtomRuntime, Priority, Purpose, Situation, Taxonomy};

#[expect(
    clippy::too_many_arguments,
    reason = "the fixture sets every atom field the composer cases vary"
)]
fn atom(
    id: &str,
    address: &str,
    order: i64,
    situations: &[Situation],
    runtime: AtomRuntime,
    priority: Priority,
    purpose: Purpose,
    taxonomy: Taxonomy,
    requires: Option<Vec<&str>>,
) -> Atom {
    Atom {
        id: id.to_owned(),
        address: address.to_owned(),
        taxonomy,
        regions: None,
        situations: situations.to_vec(),
        purpose,
        runtime,
        order,
        requires: requires.map(|ids| ids.iter().map(|id| (*id).to_owned()).collect()),
        priority,
        profile_only: false,
        probe: None,
    }
}

fn files(refer: &str) -> Result<String, ReadFail> {
    let text = match refer {
        "ontology.md" => {
            "## Identity\nwho you are\n## Affordances\nwhat you can do\n### Verbs\nsend and capture"
        }
        "recap.md" => "## Recent Decisions\nwe chose X because Y",
        "walk.md" => "## Welcome\nhello",
        _ => return Err(ReadFail::Failed(format!("no such file {refer}"))),
    };
    Ok(text.to_owned())
}

fn graph() -> Vec<Atom> {
    vec![
        atom(
            "welcome",
            "walk.md#welcome",
            1,
            &[Situation::Fresh],
            AtomRuntime::Any,
            Priority::Core,
            Purpose::Depth,
            Taxonomy::World,
            None,
        ),
        atom(
            "identity",
            "ontology.md#identity",
            2,
            &[Situation::Fresh],
            AtomRuntime::Any,
            Priority::Core,
            Purpose::Depth,
            Taxonomy::World,
            None,
        ),
        atom(
            "affordances",
            "ontology.md#affordances",
            3,
            &[Situation::Fresh, Situation::PostCompaction],
            AtomRuntime::Any,
            Priority::Core,
            Purpose::Width,
            Taxonomy::World,
            Some(vec!["identity"]),
        ),
        atom(
            "recap",
            "recap.md#recent-decisions",
            9,
            &[Situation::Handover],
            AtomRuntime::Any,
            Priority::Core,
            Purpose::Depth,
            Taxonomy::Lore,
            None,
        ),
    ]
}

fn ids(
    atoms: &[Atom],
    situation: Situation,
    runtime: crewmux_spec::context::ProfileRuntime,
) -> Vec<String> {
    let mut read = files;
    compose_profile(atoms, situation, runtime, None, &mut read, None)
        .expect("compose")
        .pieces
        .into_iter()
        .map(|piece| piece.atom_id)
        .collect()
}

#[test]
fn fresh_is_the_base_walk_and_handover_adds_handover_material() {
    let atoms = graph();
    assert_eq!(
        ids(
            &atoms,
            Situation::Fresh,
            crewmux_spec::context::ProfileRuntime::Claude
        ),
        ["welcome", "identity", "affordances"]
    );
    assert_eq!(
        ids(
            &atoms,
            Situation::Handover,
            crewmux_spec::context::ProfileRuntime::Claude
        ),
        ["welcome", "identity", "affordances", "recap"]
    );
}

#[test]
fn post_compaction_is_the_tagged_subset_closed_over_requires() {
    assert_eq!(
        ids(
            &graph(),
            Situation::PostCompaction,
            crewmux_spec::context::ProfileRuntime::Claude
        ),
        ["identity", "affordances", "recap"]
    );
}

#[test]
fn pieces_carry_the_resolved_span_and_a_source_label() {
    let mut read = files;
    let composed = compose_profile(
        &graph(),
        Situation::PostCompaction,
        crewmux_spec::context::ProfileRuntime::Claude,
        None,
        &mut read,
        None,
    )
    .expect("compose");
    let aff = composed
        .pieces
        .iter()
        .find(|piece| piece.atom_id == "affordances")
        .expect("aff");
    assert!(aff.text.contains("what you can do"));
    assert!(aff.text.contains("### Verbs"));
    assert_eq!(aff.source_kind, "library");
    let mut read = files;
    let labelled = compose_profile(
        &graph(),
        Situation::PostCompaction,
        crewmux_spec::context::ProfileRuntime::Claude,
        None,
        &mut read,
        Some(&|atom| {
            if atom.taxonomy == Taxonomy::Lore {
                "seat".to_owned()
            } else {
                "library".to_owned()
            }
        }),
    )
    .expect("label");
    let recap = labelled
        .pieces
        .iter()
        .find(|piece| piece.atom_id == "recap")
        .expect("recap");
    assert_eq!(recap.source_kind, "seat");
}

#[test]
fn claude_and_codex_compose_different_profiles() {
    let mut atoms = graph();
    atoms.push(atom(
        "claude-only",
        "walk.md",
        4,
        &[Situation::Fresh],
        AtomRuntime::Claude,
        Priority::Core,
        Purpose::Depth,
        Taxonomy::World,
        None,
    ));
    atoms.push(atom(
        "codex-only",
        "walk.md",
        5,
        &[Situation::Fresh],
        AtomRuntime::Codex,
        Priority::Core,
        Purpose::Depth,
        Taxonomy::World,
        None,
    ));
    let claude = ids(
        &atoms,
        Situation::Fresh,
        crewmux_spec::context::ProfileRuntime::Claude,
    );
    let codex = ids(
        &atoms,
        Situation::Fresh,
        crewmux_spec::context::ProfileRuntime::Codex,
    );
    assert!(claude.contains(&"claude-only".to_owned()));
    assert!(!claude.contains(&"codex-only".to_owned()));
    assert!(codex.contains(&"codex-only".to_owned()));
    assert!(!codex.contains(&"claude-only".to_owned()));
}

#[test]
fn requires_closure_across_runtime_filter_fails_loud() {
    let atoms = vec![
        atom(
            "base",
            "walk.md",
            1,
            &[Situation::Fresh],
            AtomRuntime::Codex,
            Priority::Core,
            Purpose::Depth,
            Taxonomy::World,
            None,
        ),
        atom(
            "top",
            "ontology.md#identity",
            2,
            &[Situation::Fresh],
            AtomRuntime::Any,
            Priority::Core,
            Purpose::Depth,
            Taxonomy::World,
            Some(vec!["base"]),
        ),
    ];
    let mut read = files;
    let err = compose_profile(
        &atoms,
        Situation::Fresh,
        crewmux_spec::context::ProfileRuntime::Claude,
        None,
        &mut read,
        None,
    )
    .expect_err("runtime");
    let message = err.message.to_ascii_lowercase();
    assert!(message.contains("base") && message.contains("runtime"));
}

#[test]
fn composes_a_real_section_after_a_fenced_example() {
    static N: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "rig-fence-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).expect("dir");
    fs::write(
        root.join("walk.md"),
        "## Example\n```markdown\n```language\n## Example-only heading\n```\n## Instructions\ncontinue with the real instructions",
    )
    .expect("walk");
    let roots = SourceRoots::default();
    let mut read = |refer: &str| read_source(&root, &roots, refer).map(|(text, _)| text);
    let atom = atom(
        "instructions",
        "walk.md#instructions",
        0,
        &[Situation::Fresh],
        AtomRuntime::Any,
        Priority::Core,
        Purpose::Depth,
        Taxonomy::World,
        None,
    );
    let composed = compose_profile(
        &[atom],
        Situation::Fresh,
        crewmux_spec::context::ProfileRuntime::Claude,
        None,
        &mut read,
        None,
    )
    .expect("fence");
    assert_eq!(
        composed.pieces.first().expect("piece").text,
        "## Instructions\ncontinue with the real instructions"
    );
    drop(fs::remove_dir_all(root));
}

#[test]
fn missing_file_stops_the_compose_naming_the_atom() {
    let atoms = vec![atom(
        "ghosty",
        "ghost.md#nope",
        1,
        &[Situation::Fresh],
        AtomRuntime::Any,
        Priority::Core,
        Purpose::Depth,
        Taxonomy::World,
        None,
    )];
    let mut read = files;
    let err = compose_profile(
        &atoms,
        Situation::Fresh,
        crewmux_spec::context::ProfileRuntime::Claude,
        None,
        &mut read,
        None,
    )
    .expect_err("missing");
    assert!(err.message.contains("ghosty"));
}

#[test]
fn address_matching_nothing_stops_the_compose() {
    let atoms = vec![atom(
        "misaddressed",
        "walk.md#not-there",
        1,
        &[Situation::Fresh],
        AtomRuntime::Any,
        Priority::Core,
        Purpose::Depth,
        Taxonomy::World,
        None,
    )];
    let mut read = files;
    let err = compose_profile(
        &atoms,
        Situation::Fresh,
        crewmux_spec::context::ProfileRuntime::Claude,
        None,
        &mut read,
        None,
    )
    .expect_err("miss");
    assert!(err.message.contains("misaddressed") || err.message.contains("not-there"));
}

fn absent_fixture() -> (std::path::PathBuf, SourceRoots, Atom, Atom) {
    static N: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "rig-absent-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let seat = root.join("seat");
    fs::create_dir_all(&seat).expect("seat");
    fs::write(root.join("walk.md"), "## Welcome\nhello").expect("walk");
    let roots = SourceRoots {
        seat: Some(seat),
        ..SourceRoots::default()
    };
    let welcome = atom(
        "welcome",
        "walk.md#welcome",
        1,
        &[Situation::PostCompaction],
        AtomRuntime::Any,
        Priority::Core,
        Purpose::Depth,
        Taxonomy::World,
        None,
    );
    let recap = atom(
        "recap",
        "seat:RECAP.md#decisions",
        9,
        &[Situation::Handover, Situation::PostCompaction],
        AtomRuntime::Any,
        Priority::Core,
        Purpose::Width,
        Taxonomy::Lore,
        None,
    );
    (root, roots, welcome, recap)
}

fn compose_absent(
    root: &std::path::Path,
    roots: &SourceRoots,
    atoms: &[Atom],
    situation: Situation,
) -> Result<ComposedProfile, ComposeError> {
    let mut read = |refer: &str| read_source(root, roots, refer).map(|(text, _)| text);
    compose_profile(
        atoms,
        situation,
        crewmux_spec::context::ProfileRuntime::Claude,
        None,
        &mut read,
        None,
    )
}

#[test]
fn post_compaction_skips_only_a_genuinely_absent_seat_recap() {
    let (root, roots, welcome, recap) = absent_fixture();
    let skipped = compose_absent(
        &root,
        &roots,
        &[welcome.clone(), recap.clone()],
        Situation::PostCompaction,
    )
    .expect("skip");
    assert_eq!(
        skipped
            .pieces
            .iter()
            .map(|piece| piece.atom_id.as_str())
            .collect::<Vec<_>>(),
        ["welcome"]
    );
    assert_eq!(
        skipped.skipped,
        vec![("recap".to_owned(), "seat:RECAP.md#decisions".to_owned())]
    );
    let handover = compose_absent(
        &root,
        &roots,
        &[welcome.clone(), recap.clone()],
        Situation::Handover,
    )
    .expect_err("handover");
    assert!(handover.message.contains("recap"));
    let lore = atom(
        "lore",
        "seat:LORE.md",
        5,
        &[Situation::PostCompaction],
        AtomRuntime::Any,
        Priority::Core,
        Purpose::Depth,
        Taxonomy::Lore,
        None,
    );
    let lore_err = compose_absent(
        &root,
        &roots,
        &[welcome.clone(), lore],
        Situation::PostCompaction,
    )
    .expect_err("lore");
    assert!(lore_err.message.contains("lore"));
    let missing_root = SourceRoots {
        seat: Some(root.join("no-such-seat")),
        ..SourceRoots::default()
    };
    let root_err = compose_absent(
        &root,
        &missing_root,
        &[welcome.clone(), recap.clone()],
        Situation::PostCompaction,
    )
    .expect_err("root");
    assert!(root_err.message.contains("recap"));
    let seat = roots.seat.clone().expect("seat");
    fs::write(seat.join("RECAP.md"), "## Decisions\nkept").expect("recap");
    let present = compose_absent(&root, &roots, &[welcome, recap], Situation::PostCompaction)
        .expect("present");
    assert_eq!(
        present
            .pieces
            .iter()
            .map(|piece| piece.atom_id.as_str())
            .collect::<Vec<_>>(),
        ["welcome", "recap"]
    );
    assert_eq!(present.skipped, []);
    drop(fs::remove_dir_all(root));
}

#[test]
fn over_budget_reports_drop_candidates_and_truncates_nothing() {
    let atoms = vec![
        atom(
            "must",
            "ontology.md#identity",
            1,
            &[Situation::Fresh],
            AtomRuntime::Any,
            Priority::Core,
            Purpose::Depth,
            Taxonomy::World,
            None,
        ),
        atom(
            "nice",
            "ontology.md#affordances",
            2,
            &[Situation::Fresh],
            AtomRuntime::Any,
            Priority::Recommended,
            Purpose::Width,
            Taxonomy::World,
            None,
        ),
        atom(
            "extra",
            "recap.md#recent-decisions",
            3,
            &[Situation::Fresh],
            AtomRuntime::Any,
            Priority::Optional,
            Purpose::Depth,
            Taxonomy::Lore,
            None,
        ),
    ];
    let mut read = files;
    let composed = compose_profile(
        &atoms,
        Situation::Fresh,
        crewmux_spec::context::ProfileRuntime::Claude,
        Some(1),
        &mut read,
        None,
    )
    .expect("budget");
    assert_eq!(composed.pieces.len(), 3);
    let budget = composed.budget.expect("over");
    assert_eq!(budget.limit_tokens, 1);
    assert!(budget.overage_tokens > 0);
    assert_eq!(
        budget
            .drop_candidates
            .iter()
            .map(|item| item.atom_id.as_str())
            .collect::<Vec<_>>(),
        ["extra", "nice", "must"]
    );
}

#[test]
fn within_budget_has_no_budget_report() {
    let atoms = vec![atom(
        "small",
        "walk.md#welcome",
        1,
        &[Situation::Fresh],
        AtomRuntime::Any,
        Priority::Core,
        Purpose::Depth,
        Taxonomy::World,
        None,
    )];
    let mut read = files;
    let composed = compose_profile(
        &atoms,
        Situation::Fresh,
        crewmux_spec::context::ProfileRuntime::Claude,
        Some(100_000),
        &mut read,
        None,
    )
    .expect("budget");
    assert!(composed.budget.is_none());
    assert!(composed.total_estimated_tokens > 0);
}
