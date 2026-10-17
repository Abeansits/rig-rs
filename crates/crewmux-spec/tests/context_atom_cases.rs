//! One case per `context-pack-atoms.test.ts` title.
#![expect(clippy::expect_used, reason = "parse failures abort the case")]

use std::fmt::Write;

use crewmux_spec::context::parse_manifest;

const BASE: &str = "
name: world-install
version: \"1\"
taxonomy: world
files:
  - { path: 04-ontology.md, role: world }
  - { path: what-you-can-do.md, role: world }
  - { path: probes.yaml, role: probes }
";

fn with_atoms(atoms: &str) -> String {
    format!("{BASE}atoms:\n{atoms}")
}

fn stub(over: &str) -> String {
    with_atoms(&format!(
        "  - id: a1\n    address: 04-ontology.md\n    taxonomy: world\n    situations: [fresh]\n    purpose: depth\n    order: 1\n    priority: core\n{over}"
    ))
}

fn message(yaml: &str) -> String {
    parse_manifest(yaml, "m.yaml").expect_err("reject").message
}

#[test]
fn manifest_without_atoms_still_parses() {
    let manifest = parse_manifest(BASE, "m.yaml").expect("bare");
    assert_eq!(manifest.atoms, []);
}

#[test]
fn valid_atom_round_trips_every_schema_field() {
    let yaml = with_atoms(
        "
  - id: ontology-width
    address: \"04-ontology.md#affordances\"
    taxonomy: world
    regions: [affordances, terrain]
    situations: [fresh, post-compaction]
    purpose: width
    runtime: claude
    order: 10
    priority: core
    probe:
      prompt: \"A seat asks what verbs exist for reaching a peer.\"
      expect: \"Names rig send/capture without re-reading the install.\"
",
    );
    let atom = &parse_manifest(&yaml, "m.yaml").expect("atom").atoms[0];
    assert_eq!(atom.id, "ontology-width");
    assert_eq!(atom.address, "04-ontology.md#affordances");
    assert_eq!(atom.taxonomy.as_str(), "world");
    assert_eq!(
        atom.regions.as_ref().map(|regions| regions
            .iter()
            .map(|region| region.as_str())
            .collect::<Vec<_>>()),
        Some(vec!["affordances", "terrain"])
    );
    assert_eq!(
        atom.situations
            .iter()
            .map(|item| item.as_str())
            .collect::<Vec<_>>(),
        ["fresh", "post-compaction"]
    );
    assert_eq!(atom.purpose.as_str(), "width");
    assert_eq!(atom.runtime.as_str(), "claude");
    assert_eq!(atom.order, 10);
    assert_eq!(atom.priority.as_str(), "core");
    let probe = atom.probe.as_ref().expect("probe");
    assert_eq!(
        probe.prompt,
        "A seat asks what verbs exist for reaching a peer."
    );
    assert_eq!(
        probe.expect,
        "Names rig send/capture without re-reading the install."
    );
}

#[test]
fn defaults_runtime_any_and_optional_regions_requires_probe() {
    let yaml = with_atoms(
        "
  - id: minimal
    address: what-you-can-do.md
    taxonomy: world
    situations: [fresh]
    purpose: width
    order: 20
    priority: recommended
",
    );
    let atom = &parse_manifest(&yaml, "m.yaml").expect("atom").atoms[0];
    assert_eq!(atom.runtime.as_str(), "any");
    assert!(atom.regions.is_none());
    assert!(atom.requires.is_none());
    assert!(atom.probe.is_none());
}

#[test]
fn whole_file_atom_allows_any_declared_file_header_needs_markdown() {
    let whole = with_atoms(
        "
  - id: probes-file
    address: probes.yaml
    taxonomy: skills
    situations: [fresh]
    purpose: depth
    order: 30
    priority: optional
",
    );
    assert_eq!(
        parse_manifest(&whole, "m.yaml").expect("whole").atoms[0].address,
        "probes.yaml"
    );
    let bad = with_atoms(
        "
  - id: bad
    address: \"probes.yaml#some-header\"
    taxonomy: skills
    situations: [fresh]
    purpose: depth
    order: 31
    priority: optional
",
    );
    let err = message(&bad);
    assert!(err.contains("header") || err.to_ascii_lowercase().contains("markdown"));
}

#[test]
fn rejects_a_duplicate_atom_id() {
    let err = message(&stub(
        "
  - id: a1
    address: what-you-can-do.md
    taxonomy: world
    situations: [fresh]
    purpose: width
    order: 2
    priority: core
",
    ));
    assert!(err.to_ascii_lowercase().contains("duplicate"));
    assert!(err.contains("a1"));
}

#[test]
fn rejects_undeclared_requires_and_self_requires() {
    let ghost = message(&stub(
        "
  - id: a2
    address: what-you-can-do.md
    taxonomy: world
    situations: [fresh]
    purpose: width
    order: 2
    priority: core
    requires: [ghost]
",
    ));
    assert!(ghost.contains("ghost"));
    let itself = message(&with_atoms(
        "
  - id: selfy
    address: 04-ontology.md
    taxonomy: world
    situations: [fresh]
    purpose: depth
    order: 1
    priority: core
    requires: [selfy]
",
    ));
    let lower = itself.to_ascii_lowercase();
    assert!(lower.contains("itself") || lower.contains("self"));
}

#[test]
fn rejects_a_requires_cycle() {
    let err = message(&with_atoms(
        "
  - id: a
    address: 04-ontology.md
    taxonomy: world
    situations: [fresh]
    purpose: depth
    order: 1
    priority: core
    requires: [b]
  - id: b
    address: what-you-can-do.md
    taxonomy: world
    situations: [fresh]
    purpose: width
    order: 2
    priority: core
    requires: [a]
",
    ));
    assert!(err.to_ascii_lowercase().contains("cycle"));
}

#[test]
fn tree_prefixed_addresses_are_legal_at_ingest() {
    let yaml = with_atoms(
        "
  - id: project-intent
    address: \"project:SPEC.md\"
    taxonomy: mission
    situations: [fresh]
    purpose: depth
    order: 1
    priority: core
  - id: recap
    address: \"seat:RECAP.md#recent-decisions\"
    taxonomy: lore
    situations: [handover]
    purpose: width
    order: 90
    priority: core
",
    );
    let atoms = parse_manifest(&yaml, "m.yaml").expect("tree").atoms;
    assert_eq!(
        atoms
            .iter()
            .map(|atom| atom.address.as_str())
            .collect::<Vec<_>>(),
        ["project:SPEC.md", "seat:RECAP.md#recent-decisions"]
    );
    let traversal = message(&with_atoms(
        "
  - id: sneaky
    address: \"seat:../LEARNED.md\"
    taxonomy: lore
    situations: [handover]
    purpose: width
    order: 91
    priority: core
",
    ));
    assert!(traversal.to_ascii_lowercase().contains("traversal"));
    let markdown = message(&with_atoms(
        "
  - id: notmd
    address: \"mission:data.yaml#x\"
    taxonomy: mission
    situations: [fresh]
    purpose: depth
    order: 92
    priority: core
",
    ));
    assert!(markdown.to_ascii_lowercase().contains("markdown"));
}

#[test]
fn rejects_an_address_whose_ref_is_not_a_declared_pack_file() {
    let err = message(&with_atoms(
        "
  - id: stray
    address: \"not-in-pack.md#x\"
    taxonomy: world
    situations: [fresh]
    purpose: depth
    order: 1
    priority: core
",
    ));
    assert!(err.contains("not-in-pack.md"));
}

#[test]
fn rejects_a_malformed_address_via_the_one_grammar() {
    let err = message(&with_atoms(
        "
  - id: bad
    address: \"04-ontology.md#a#b\"
    taxonomy: world
    situations: [fresh]
    purpose: depth
    order: 1
    priority: core
",
    ));
    assert!(err.contains('#'));
}

#[test]
fn deep_requires_chain_stays_inside_the_fail_loud_channel() {
    const N: usize = 8000;
    let mut entries = String::new();
    for index in 0..N {
        write!(
            entries,
            "  - id: a{index}\n    address: 04-ontology.md\n    taxonomy: world\n    situations: [fresh]\n    purpose: depth\n    order: {index}\n    priority: core"
        )
        .expect("entry");
        if index + 1 < N {
            let next = index + 1;
            write!(entries, "\n    requires: [a{next}]\n").expect("requires");
        } else {
            entries.push('\n');
        }
    }
    let manifest = parse_manifest(&with_atoms(&entries), "m.yaml").expect("deep");
    assert_eq!(manifest.atoms.len(), N);
    let cyclic = format!(
        "{entries}  - id: z\n    address: 04-ontology.md\n    taxonomy: world\n    situations: [fresh]\n    purpose: depth\n    order: {N}\n    priority: core\n    requires: [z2]\n  - id: z2\n    address: 04-ontology.md\n    taxonomy: world\n    situations: [fresh]\n    purpose: depth\n    order: {}\n    priority: core\n    requires: [z]\n",
        N + 1
    );
    let err = message(&with_atoms(&cyclic));
    assert!(err.to_ascii_lowercase().contains("cycle"));
}

#[test]
fn unknown_atom_key_rejects_loud() {
    let require = message(&stub(
        "
  - id: a2
    address: what-you-can-do.md
    taxonomy: world
    situations: [fresh]
    purpose: width
    order: 2
    priority: core
    require: [a1]
",
    ));
    assert!(require.contains("unknown field 'require'"));
    let probes = message(&with_atoms(
        "
  - id: x
    address: 04-ontology.md
    taxonomy: world
    situations: [fresh]
    purpose: depth
    order: 1
    priority: core
    probes: { prompt: p, expect: e }
",
    ));
    assert!(probes.contains("unknown field 'probes'"));
}

#[test]
fn rejects_bad_enums_and_shapes() {
    let cases = [
        ("taxonomy: cosmos", "taxonomy"),
        ("situations: []", "situations"),
        ("purpose: girth", "purpose"),
        ("runtime: gemini", "runtime"),
        ("order: 1.5", "order"),
        ("priority: urgent", "priority"),
        ("probe: { prompt: only-half }", "probe"),
    ];
    for (line, want) in cases {
        let key = line.split(':').next().expect("key");
        let mut fields = [
            ("taxonomy", "taxonomy: world"),
            ("situations", "situations: [fresh]"),
            ("purpose", "purpose: depth"),
            ("runtime", ""),
            ("order", "order: 1"),
            ("priority", "priority: core"),
            ("probe", ""),
        ];
        for field in &mut fields {
            if field.0 == key {
                field.1 = line;
            }
        }
        let yaml = with_atoms(&format!(
            "  - id: x\n    address: 04-ontology.md\n    {}\n    {}\n    {}\n    {}\n    {}\n    {}\n    {}\n",
            fields[0].1,
            fields[1].1,
            fields[2].1,
            fields[3].1,
            fields[4].1,
            fields[5].1,
            fields[6].1
        ));
        let err = message(&yaml);
        assert!(err.contains(want), "{line}: {err}");
    }
}
