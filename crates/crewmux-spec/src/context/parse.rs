// Derived from openrig packages/daemon/src/domain/context-packs/manifest-parser.ts
// (v0.6.9, Apache-2.0); ported to Rust and modified.

//! `parse_manifest`: YAML object to [`PackManifest`](super::PackManifest).

use serde_json::{Number, Value};

use super::atoms::parse_atoms;
use super::error::{ManifestCode, ManifestError};
use super::files::parse_files;
use super::load::load_yaml;
use super::profiles::parse_profiles;
use super::safety::is_safe_version;
use super::types::{PackManifest, TAXONOMY_TEACHING, Taxonomy};

/// Parse one context-pack manifest.
///
/// # Errors
///
/// Returns [`ManifestError`] with the TypeScript code and sentence.
pub fn parse_manifest(raw_yaml: &str, source_path: &str) -> Result<PackManifest, ManifestError> {
    let parsed = load_yaml(raw_yaml, source_path)?;
    let Some(object) = parsed.as_object() else {
        return Err(invalid(format!(
            "manifest at {source_path} must be a YAML object at the root"
        )));
    };
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| {
            invalid(format!(
                "manifest at {source_path} is missing required field 'name' (string)"
            ))
        })?
        .to_owned();
    let version = version_of(object.get("version"), source_path)?;
    let description = object
        .get("purpose")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let taxonomy = taxonomy_of(object.get("taxonomy"), source_path)?;
    let files = parse_files(object.get("files"), source_path)?;
    let estimated_tokens = object
        .get("estimated_tokens")
        .or_else(|| object.get("estimatedTokens"))
        .and_then(Value::as_number)
        .and_then(floor_tokens);
    let atoms = match object.get("atoms") {
        Some(raw) => parse_atoms(raw, &files, source_path)?,
        None => Vec::new(),
    };
    let profiles = match object.get("profiles") {
        Some(raw) => parse_profiles(raw, &atoms, source_path)?,
        None => Vec::new(),
    };
    Ok(PackManifest {
        name,
        version,
        description,
        taxonomy,
        files,
        estimated_tokens,
        atoms,
        profiles,
    })
}

fn version_of(raw: Option<&Value>, source_path: &str) -> Result<String, ManifestError> {
    let Some(raw) = raw.filter(|value| !value.is_null()) else {
        return Err(invalid(format!(
            "manifest at {source_path} is missing required field 'version'"
        )));
    };
    let version = js_string(raw);
    if !is_safe_version(&version) {
        return Err(invalid(format!(
            "manifest at {source_path} has an invalid version '{version}' — a version must be a single bounded token \
             [A-Za-z0-9][A-Za-z0-9._+-]{{0,31}} (no ':' or other separator, no whitespace, ≤32 chars) so it cannot \
             forge a '<name>:<version>' store id or breach the OS filename bound."
        )));
    }
    Ok(version)
}

fn taxonomy_of(raw: Option<&Value>, source_path: &str) -> Result<Taxonomy, ManifestError> {
    let Some(raw) = raw.filter(|value| !value.is_null()) else {
        return Err(invalid(format!(
            "manifest at {source_path} is missing required field 'taxonomy' — every context pack declares what kind of context it is. {TAXONOMY_TEACHING}"
        )));
    };
    let Some(text) = raw.as_str() else {
        return Err(invalid(format!(
            "manifest at {source_path} has invalid taxonomy {}. {TAXONOMY_TEACHING}",
            js_json(raw)
        )));
    };
    Taxonomy::parse(text).ok_or_else(|| {
        invalid(format!(
            "manifest at {source_path} has invalid taxonomy {}. {TAXONOMY_TEACHING}",
            js_json(raw)
        ))
    })
}

fn js_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Bool(true) => "true".to_owned(),
        Value::Bool(false) => "false".to_owned(),
        Value::Number(number) => js_number(number),
        Value::Array(items) => items.iter().map(js_string).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".to_owned(),
        Value::Null => "null".to_owned(),
    }
}

fn js_number(number: &Number) -> String {
    if let Some(value) = number.as_i64() {
        return value.to_string();
    }
    if let Some(value) = number.as_u64() {
        return value.to_string();
    }
    let Some(value) = number.as_f64() else {
        return number.to_string();
    };
    if value.is_finite() && value.fract() == 0.0 {
        return format!("{value:.0}");
    }
    number.to_string()
}

fn floor_tokens(number: &Number) -> Option<u32> {
    let value = number.as_f64()?;
    if !value.is_finite() {
        return None;
    }
    let floored = value.floor();
    if floored <= 0.0 {
        return Some(0);
    }
    format!("{floored:.0}").parse().ok()
}

fn js_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned())
}

fn invalid(message: String) -> ManifestError {
    ManifestError::new(ManifestCode::Invalid, message)
}

#[cfg(test)]
mod tests {
    use super::parse_manifest;

    const VALID: &str = "
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

    #[test]
    fn parses_a_valid_manifest() {
        let manifest = parse_manifest(VALID, "/test/manifest.yaml").expect("parse");
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
        assert_eq!(manifest.files[0].summary.as_deref(), Some("Phase A PRD"));
        assert!(manifest.files[1].summary.is_none());
        assert_eq!(manifest.estimated_tokens, Some(500));
    }

    #[test]
    fn normalizes_numeric_versions() {
        let manifest = parse_manifest("name: x\nversion: 2\ntaxonomy: world\nfiles: []", "/x.yaml")
            .expect("parse");
        assert_eq!(manifest.version, "2");
    }

    #[test]
    fn rejects_bad_yaml_missing_fields_and_paths() {
        let err = parse_manifest("{not valid", "/x.yaml").expect_err("yaml");
        assert_eq!(err.code.as_str(), "manifest_parse_error");
        assert!(
            parse_manifest("version: 1\nfiles: []", "/x.yaml")
                .unwrap_err()
                .message
                .contains("name")
        );
        assert!(
            parse_manifest("name: x\nfiles: []", "/x.yaml")
                .unwrap_err()
                .message
                .contains("version")
        );
        assert!(
            parse_manifest(
                "name: x\nversion: 1\ntaxonomy: world\nfiles: not-an-array",
                "/x.yaml"
            )
            .unwrap_err()
            .message
            .contains("files")
        );
        let escape = "name: x\nversion: 1\ntaxonomy: world\nfiles:\n  - path: ../escape.md\n    role: notes\n";
        assert!(
            parse_manifest(escape, "/x.yaml")
                .unwrap_err()
                .message
                .contains("relative path inside the pack")
        );
        let abs = "name: x\nversion: 1\ntaxonomy: world\nfiles:\n  - path: /etc/passwd\n    role: notes\n";
        assert!(
            parse_manifest(abs, "/x.yaml")
                .unwrap_err()
                .message
                .contains("relative path inside the pack")
        );
        let png =
            "name: x\nversion: 1\ntaxonomy: world\nfiles:\n  - path: image.png\n    role: code\n";
        assert!(
            parse_manifest(png, "/x.yaml")
                .unwrap_err()
                .message
                .contains("unsupported suffix")
        );
        let role = "name: x\nversion: 1\ntaxonomy: world\nfiles:\n  - path: notes.md\n";
        assert!(
            parse_manifest(role, "/x.yaml")
                .unwrap_err()
                .message
                .contains("missing 'role'")
        );
    }

    #[test]
    fn accepts_allowed_suffixes_and_ignores_bad_token_estimates() {
        let ok = "name: x\nversion: 1\ntaxonomy: world\nfiles:\n  - { path: a.md, role: r }\n  - { path: b.markdown, role: r }\n  - { path: c.yaml, role: r }\n  - { path: d.yml, role: r }\n  - { path: e.txt, role: r }\n  - { path: scripts/a.sh, role: r }\n  - { path: scripts/b.ts, role: r }\n  - { path: scripts/c.mjs, role: r }\n  - { path: scripts/d.py, role: r }\n";
        assert_eq!(
            parse_manifest(ok, "/x.yaml").expect("suffixes").files.len(),
            9
        );
        let tokens = parse_manifest(
            "name: x\nversion: 1\ntaxonomy: world\nfiles: []\nestimated_tokens: 'not-a-number'",
            "/x.yaml",
        )
        .expect("tokens");
        assert_eq!(tokens.estimated_tokens, None);
    }

    #[test]
    fn rejects_unsafe_versions_and_teaches_taxonomy() {
        let colon = parse_manifest(
            "name: x\nversion: '1:0:0'\ntaxonomy: world\nfiles: []",
            "/x.yaml",
        )
        .unwrap_err();
        assert_eq!(colon.code.as_str(), "manifest_invalid");
        assert!(colon.message.contains("version"));
        let long = format!(
            "name: x\nversion: '{}'\ntaxonomy: world\nfiles: []",
            "1".repeat(33)
        );
        assert!(
            parse_manifest(&long, "/x.yaml")
                .unwrap_err()
                .message
                .contains("version")
        );
        let ok = parse_manifest(
            "name: x\nversion: 1.2.3_rc+build\ntaxonomy: world\nfiles: []",
            "/x.yaml",
        );
        assert!(ok.is_ok(), "{ok:?}");
        let missing = parse_manifest("name: x\nversion: 1\nfiles: []", "/x.yaml").unwrap_err();
        assert!(missing.message.contains("taxonomy"));
        assert!(missing.message.contains("world | lore | skills | mission"));
        let bad = parse_manifest(
            "name: x\nversion: 1\ntaxonomy: doctrine\nfiles: []",
            "/x.yaml",
        )
        .unwrap_err();
        assert!(bad.message.contains("doctrine"));
        assert!(bad.message.contains(super::TAXONOMY_TEACHING));
    }

    #[test]
    fn applies_merge_keys() {
        let yaml = "base: &base\n  role: notes\nname: x\nversion: \"1\"\ntaxonomy: skills\nfiles:\n  - <<: *base\n    path: a.md\n";
        let manifest = parse_manifest(yaml, "/x.yaml").expect("merge");
        assert_eq!(manifest.files[0].role, "notes");
        assert_eq!(manifest.files[0].path, "a.md");
    }
}
