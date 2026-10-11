//! Codex `config.toml` additions stay explicit and reversible.
#![expect(
    clippy::unwrap_used,
    reason = "round-trip tests assert writer results directly"
)]

use std::fs;

use crewmux_provider::{CodexConfig, CodexHealth, ProviderError, TrustLevel, classify_config};

const SAMPLE: &str = include_str!("fixtures/config.sample.toml");

fn loaded(dir: &std::path::Path) -> (std::path::PathBuf, CodexConfig) {
    let path = dir.join("config.toml");
    fs::write(&path, SAMPLE).unwrap();
    let config = CodexConfig::load(&path).unwrap();
    (path, config)
}

#[test]
fn additions_only() {
    let dir = tempfile::tempdir().unwrap();
    let (_path, mut config) = loaded(dir.path());
    let project = std::path::PathBuf::from("/work/app");
    assert_eq!(
        config
            .ensure_mcp_server("rig", &["rig".to_owned(), "mcp".to_owned()])
            .unwrap(),
        crewmux_provider::Change::Added
    );
    assert_eq!(
        config
            .ensure_project_trust(&project, TrustLevel::Trusted)
            .unwrap(),
        crewmux_provider::Change::Added
    );
    let rendered = config.rendered();
    assert!(rendered.starts_with(SAMPLE.trim_end()));
    assert!(rendered.contains("# Codex sample from the config-writes spike."));
    assert!(rendered.contains("# inline comment on the model"));
    assert!(rendered.contains("# profile keeps its comment"));
    assert!(rendered.contains("# existing server stays"));
    assert!(rendered.contains("# trailing comment stays"));
    assert!(rendered.contains("[mcp_servers.rig]"));
    assert!(rendered.contains("[projects.\"/work/app\"]"));
    assert!(rendered.contains("trust_level = \"trusted\""));
    for line in rendered.lines() {
        assert_ne!(line.trim(), "[projects]");
        assert!(!line.contains("projects = {"));
    }
}

#[test]
fn idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let (_path, mut config) = loaded(dir.path());
    let project = std::path::PathBuf::from("/work/app");
    config
        .ensure_mcp_server("rig", &["rig".to_owned(), "mcp".to_owned()])
        .unwrap();
    config
        .ensure_project_trust(&project, TrustLevel::Trusted)
        .unwrap();
    let once = config.rendered().to_owned();
    assert_eq!(
        config
            .ensure_mcp_server("rig", &["rig".to_owned(), "mcp".to_owned()])
            .unwrap(),
        crewmux_provider::Change::Unchanged
    );
    assert_eq!(
        config
            .ensure_project_trust(&project, TrustLevel::Trusted)
            .unwrap(),
        crewmux_provider::Change::Unchanged
    );
    assert_eq!(config.rendered(), once);
}

#[test]
fn remove_restores_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let (path, mut config) = loaded(dir.path());
    config
        .ensure_mcp_server("rig", &["rig".to_owned(), "mcp".to_owned()])
        .unwrap();
    config
        .ensure_project_trust(std::path::Path::new("/work/app"), TrustLevel::Trusted)
        .unwrap();
    assert_eq!(config.remove_ours(), crewmux_provider::Change::Removed);
    assert_eq!(config.rendered(), SAMPLE);
    let backups = crewmux_provider::Backups::new(&dir.path().join("backups"));
    config.save(&backups).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), SAMPLE);
}

#[test]
fn invalid_toml_is_not_written() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    fs::write(&path, "this = [\n").unwrap();
    let error = CodexConfig::load(&path).unwrap_err();
    assert!(matches!(error, ProviderError::Parse { .. }));
    assert_eq!(fs::read_to_string(&path).unwrap(), "this = [\n");
    let classified = classify_config("this = [\n").unwrap_err();
    assert!(matches!(classified, ProviderError::Parse { .. }));
    assert_eq!(
        classify_config("[mcp_servers.rig]\ncommand = \"other\"\n").unwrap(),
        CodexHealth::Drift
    );
}
