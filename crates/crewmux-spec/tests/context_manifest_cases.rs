//! One case per `context-pack-manifest-parser.test.ts` title.
#![expect(clippy::expect_used, reason = "parse failures abort the case")]

use crewmux_spec::context::{ManifestCode, parse_manifest};

fn message(yaml: &str) -> String {
    parse_manifest(yaml, "/x.yaml").expect_err("reject").message
}

#[test]
fn parses_a_valid_manifest_into_the_typed_shape() {
    let yaml = "
name: pl-005-priming
version: 1
taxonomy: mission
purpose: Priming for PL-005 Phase A
files:
  - path: prd.md
    role: prd
    summary: Phase A PRD
  - path: proof.md
    role: proof-packet
estimated_tokens: 500
";
    let manifest = parse_manifest(yaml, "/test/manifest.yaml").expect("ok");
    assert_eq!(manifest.name, "pl-005-priming");
    assert_eq!(manifest.version, "1");
    assert!(
        manifest
            .purpose()
            .unwrap_or("")
            .contains("Priming for PL-005")
    );
    assert_eq!(manifest.files.len(), 2);
    assert_eq!(manifest.files[0].path, "prd.md");
    assert_eq!(manifest.files[0].role, "prd");
    assert_eq!(manifest.files[0].summary.as_deref(), Some("Phase A PRD"));
    assert_eq!(manifest.files[1].path, "proof.md");
    assert_eq!(manifest.files[1].role, "proof-packet");
    assert_eq!(manifest.files[1].summary, None);
    assert_eq!(manifest.estimated_tokens, Some(500));
}

#[test]
fn normalizes_numeric_versions_to_strings() {
    let manifest =
        parse_manifest("name: x\nversion: 2\ntaxonomy: world\nfiles: []", "/x.yaml").expect("ok");
    assert_eq!(manifest.version, "2");
}

#[test]
fn rejects_non_yaml_with_manifest_parse_error() {
    let err = parse_manifest("{not valid", "/x.yaml").expect_err("yaml");
    assert_eq!(err.code, ManifestCode::Parse);
    assert_eq!(err.code.as_str(), "manifest_parse_error");
}

#[test]
fn rejects_missing_name() {
    assert!(message("version: 1\nfiles: []").contains("name"));
}

#[test]
fn rejects_missing_version() {
    assert!(message("name: x\nfiles: []").contains("version"));
}

#[test]
fn rejects_malformed_files_array() {
    assert!(message("name: x\nversion: 1\ntaxonomy: world\nfiles: not-an-array").contains("files"));
}

#[test]
fn rejects_file_entry_with_dotdot_in_path() {
    let yaml =
        "name: x\nversion: 1\ntaxonomy: world\nfiles:\n  - path: ../escape.md\n    role: notes\n";
    assert!(message(yaml).contains("relative path inside the pack"));
}

#[test]
fn rejects_file_entry_with_absolute_path() {
    let yaml =
        "name: x\nversion: 1\ntaxonomy: world\nfiles:\n  - path: /etc/passwd\n    role: notes\n";
    assert!(message(yaml).contains("relative path inside the pack"));
}

#[test]
fn rejects_file_entry_with_unsupported_suffix() {
    let yaml =
        "name: x\nversion: 1\ntaxonomy: world\nfiles:\n  - path: image.png\n    role: code\n";
    assert!(message(yaml).contains("unsupported suffix"));
}

#[test]
fn rejects_file_entry_missing_role() {
    let yaml = "name: x\nversion: 1\ntaxonomy: world\nfiles:\n  - path: notes.md\n";
    assert!(message(yaml).contains("missing 'role'"));
}

#[test]
fn accepts_allowed_suffixes() {
    let yaml = "name: x\nversion: 1\ntaxonomy: world\nfiles:\n  - { path: a.md, role: r }\n  - { path: b.markdown, role: r }\n  - { path: c.yaml, role: r }\n  - { path: d.yml, role: r }\n  - { path: e.txt, role: r }\n";
    let manifest = parse_manifest(yaml, "/x.yaml").expect("ok");
    assert_eq!(
        manifest
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        ["a.md", "b.markdown", "c.yaml", "d.yml", "e.txt"]
    );
}

#[test]
fn accepts_inert_script_helpers() {
    let yaml = "name: x\nversion: 1\ntaxonomy: world\nfiles:\n  - { path: scripts/a.sh, role: r }\n  - { path: scripts/b.ts, role: r }\n  - { path: scripts/c.mjs, role: r }\n  - { path: scripts/d.py, role: r }\n";
    let manifest = parse_manifest(yaml, "/x.yaml").expect("ok");
    assert_eq!(
        manifest
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        [
            "scripts/a.sh",
            "scripts/b.ts",
            "scripts/c.mjs",
            "scripts/d.py"
        ]
    );
}

#[test]
fn ignores_estimated_tokens_when_not_a_finite_number() {
    let manifest = parse_manifest(
        "name: x\nversion: 1\ntaxonomy: world\nfiles: []\nestimated_tokens: 'not-a-number'",
        "/x.yaml",
    )
    .expect("ok");
    assert_eq!(manifest.estimated_tokens, None);
}

#[test]
fn parses_named_install_profiles() {
    let yaml = "
name: world
version: 1
taxonomy: world
files:
  - { path: world.md, role: world }
atoms:
  - id: bootstrap
    address: world.md
    taxonomy: world
    situations: [fresh]
    purpose: width
    runtime: any
    order: 10
    priority: core
  - id: coverage-map
    address: world.md#coverage-map
    taxonomy: world
    situations: [fresh]
    purpose: width
    runtime: any
    order: 20
    priority: core
    profile_only: true
profiles:
  - id: codex-coverage
    situations: [fresh]
    runtimes: [codex]
    phases:
      - id: bootstrap
        atoms: [bootstrap]
      - id: situated-work
        context: [project, mission, seat, slice]
      - id: coverage-map
        atoms: [coverage-map]
";
    let manifest = parse_manifest(yaml, "/world/manifest.yaml").expect("profiles");
    assert!(
        manifest
            .atoms
            .iter()
            .any(|atom| atom.id == "coverage-map" && atom.profile_only)
    );
    assert_eq!(manifest.profiles.len(), 1);
    let profile = &manifest.profiles[0];
    assert_eq!(profile.id, "codex-coverage");
    assert_eq!(profile.situations[0].as_str(), "fresh");
    assert_eq!(profile.runtimes[0].as_str(), "codex");
    assert_eq!(
        profile.phases[0].atoms.clone().unwrap_or_default(),
        vec!["bootstrap".to_owned()]
    );
    let sources = profile.phases[1]
        .context
        .as_ref()
        .map(|items| items.iter().map(|item| item.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    assert_eq!(sources, ["project", "mission", "seat", "slice"]);
    assert_eq!(
        profile.phases[2].atoms.clone().unwrap_or_default(),
        vec!["coverage-map".to_owned()]
    );
}

#[test]
fn rejects_a_named_profile_that_references_an_undeclared_atom() {
    let yaml = "
name: world
version: 1
taxonomy: world
files: []
atoms:
  - id: declared
    address: project:SPEC.md
    taxonomy: mission
    situations: [fresh]
    purpose: width
    runtime: any
    order: 10
    priority: core
profiles:
  - id: codex-coverage
    situations: [fresh]
    runtimes: [codex]
    phases:
      - id: bootstrap
        atoms: [missing]
";
    let err = message(yaml);
    let lower = err.to_ascii_lowercase();
    assert!(
        lower.contains("profile") && lower.contains("missing")
            || lower.contains("atom") && lower.contains("missing")
    );
}

#[test]
fn rejects_a_named_profile_that_repeats_an_atom_or_context_source() {
    let prefix = "
name: world
version: 1
taxonomy: world
files:
  - { path: world.md, role: world }
atoms:
  - id: bootstrap
    address: world.md
    taxonomy: world
    situations: [fresh]
    purpose: width
    runtime: any
    order: 10
    priority: core
profiles:
  - id: codex-coverage
    situations: [fresh]
    runtimes: [codex]
    phases:
";
    let atom = message(&format!(
        "{prefix}      - {{ id: first, atoms: [bootstrap] }}\n      - {{ id: second, atoms: [bootstrap] }}\n"
    ));
    assert!(
        atom.contains("bootstrap") && atom.contains("more than once") || atom.contains("duplicate")
    );
    let source = message(&format!(
        "{prefix}      - {{ id: first, context: [project] }}\n      - {{ id: second, context: [project] }}\n"
    ));
    assert!(
        source.contains("project") && source.contains("more than once")
            || source.contains("duplicate")
    );
}

#[test]
fn rejects_a_colon_bearing_version() {
    let err = parse_manifest("name: x\nversion: '1:0:0'\nfiles: []", "/x.yaml").expect_err("colon");
    assert_eq!(err.code, ManifestCode::Invalid);
    assert_eq!(err.code.as_str(), "manifest_invalid");
    assert!(err.message.contains("version"));
}

#[test]
fn rejects_an_overlong_version() {
    let long = format!("1{}", "a".repeat(300));
    let err = message(&format!("name: x\nversion: '{long}'\nfiles: []"));
    assert!(err.contains("version"));
}

#[test]
fn accepts_a_bounded_delimiter_free_version() {
    let manifest = parse_manifest(
        "name: x\nversion: '1.2.0-rc.1+build_7'\ntaxonomy: world\nfiles: []",
        "/x.yaml",
    )
    .expect("ok");
    assert_eq!(manifest.version, "1.2.0-rc.1+build_7");
}
