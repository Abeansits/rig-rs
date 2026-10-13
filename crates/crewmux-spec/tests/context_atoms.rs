//! Atom schema round-trip against the v0.6.9 manifest cases.

use crewmux_spec::context::parse_manifest;

const BASE: &str = "
name: world-install
version: \"1\"
taxonomy: world
files:
  - { path: 04-ontology.md, role: world }
  - { path: what-you-can-do.md, role: world }
";

#[test]
fn atom_round_trips_schema_fields() {
    let bare = parse_manifest(BASE, "m.yaml").expect("bare");
    assert_eq!(bare.atoms, Vec::new());
    let yaml = format!(
        "{BASE}atoms:\n  - id: ontology-width\n    address: \"04-ontology.md#affordances\"\n    taxonomy: world\n    regions: [affordances, terrain]\n    situations: [fresh, post-compaction]\n    purpose: width\n    runtime: claude\n    order: 10\n    priority: core\n    probe:\n      prompt: \"A seat asks what verbs exist.\"\n      expect: \"Names rig send.\"\n"
    );
    let manifest = parse_manifest(&yaml, "m.yaml").expect("atoms");
    let atom = manifest.atoms.first().expect("atom");
    assert_eq!(atom.id, "ontology-width");
    assert_eq!(atom.address, "04-ontology.md#affordances");
    assert_eq!(atom.taxonomy.as_str(), "world");
    assert_eq!(atom.regions.as_ref().map(Vec::len), Some(2));
    assert_eq!(atom.situations.len(), 2);
    assert_eq!(atom.purpose.as_str(), "width");
    assert_eq!(atom.runtime.as_str(), "claude");
    assert_eq!(atom.order, 10);
    assert_eq!(atom.priority.as_str(), "core");
    let probe = atom.probe.as_ref().expect("probe");
    assert!(probe.prompt.contains("verbs"));
    assert!(probe.expect.contains("rig send"));
}
