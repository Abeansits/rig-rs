//! Private Herdr launch config. Cases follow `herdr-launch-config.test.ts`.
#![expect(
    clippy::unwrap_used,
    reason = "these helpers assert filesystem results for the ported cases"
)]

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

use crewmux_provider::HerdrLaunchConfig;

fn prepare(
    home: &std::path::Path,
    xdg: Option<&std::path::Path>,
    config: Option<&std::path::Path>,
    socket: &str,
    columns: Option<u32>,
) -> std::path::PathBuf {
    HerdrLaunchConfig {
        env_home: home,
        xdg_config_home: xdg,
        config_path: config,
        socket_path: socket,
        columns,
    }
    .prepare()
    .unwrap()
}

#[test]
fn prepare_matches_upstream_cases() {
    keeps_exact_config();
    preserves_home_and_xdg();
    defaults_absent();
    independent_settings();
    leaves_invalid_existing();
    replaces_without_following_symlink();
}

fn keeps_exact_config() {
    for columns in [80, 120, 159, 160, 268] {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("my herdr.toml");
        let text = "# keep my original formatting\n[ui]\nsidebar_start_collapsed = false\nsidebar_width = 31\n";
        fs::write(&source, text).unwrap();
        let mut permissions = fs::metadata(&source).unwrap().permissions();
        permissions.set_mode(0o640);
        fs::set_permissions(&source, permissions).unwrap();
        let before = fs::metadata(&source).unwrap();
        let output = prepare(
            dir.path(),
            None,
            Some(&source),
            "/owned/herdr.sock",
            Some(columns),
        );
        assert_eq!(output, source);
        assert_eq!(fs::read_to_string(&source).unwrap(), text);
        let after = fs::metadata(&source).unwrap();
        assert_eq!(after.ino(), before.ino());
        assert_eq!(after.modified().unwrap(), before.modified().unwrap());
        assert_eq!(after.mode(), before.mode());
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1);
    }
}

fn preserves_home_and_xdg() {
    for xdg in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let custom = dir.path().join("custom");
        let xdg_path = xdg.then_some(custom.as_path());
        let config_dir = if xdg {
            custom.join("herdr")
        } else {
            dir.path().join(".config").join("herdr")
        };
        fs::create_dir_all(&config_dir).unwrap();
        let source = config_dir.join("config.toml");
        fs::write(&source, "[ui]\nsidebar_width = 23\n").unwrap();
        let output = prepare(dir.path(), xdg_path, None, "/owned.sock", Some(80));
        assert_eq!(output, source);
        assert_eq!(
            fs::read_to_string(&source).unwrap(),
            "[ui]\nsidebar_width = 23\n"
        );
    }
}

fn defaults_absent() {
    let cases = [
        (None, true),
        (Some(80), true),
        (Some(159), true),
        (Some(160), false),
        (Some(268), false),
    ];
    for (columns, collapsed) in cases {
        let dir = tempfile::tempdir().unwrap();
        let output = prepare(dir.path(), None, None, "/owned/herdr.sock", columns);
        let text = fs::read_to_string(&output).unwrap();
        assert!(text.contains("onboarding = false"), "{text}");
        assert!(
            text.contains(&format!("sidebar_start_collapsed = {collapsed}")),
            "{text}"
        );
        assert_eq!(fs::metadata(&output).unwrap().mode() & 0o777, 0o600);
        let personal = output.parent().unwrap().join("config.toml");
        assert!(!personal.exists());
        let again = prepare(dir.path(), None, None, "/owned/herdr.sock", columns);
        assert_eq!(again, output);
    }
}

fn independent_settings() {
    let dir = tempfile::tempdir().unwrap();
    let narrow = prepare(dir.path(), None, None, "/one.sock", Some(80));
    let wide = prepare(dir.path(), None, None, "/one.sock", Some(160));
    let other = prepare(dir.path(), None, None, "/two.sock", Some(160));
    assert_ne!(narrow, wide);
    assert_ne!(wide, other);
    assert!(
        fs::read_to_string(&narrow)
            .unwrap()
            .contains("sidebar_start_collapsed = true")
    );
    assert!(
        fs::read_to_string(&wide)
            .unwrap()
            .contains("sidebar_start_collapsed = false")
    );
}

fn leaves_invalid_existing() {
    for text in ["[ui\n", "ui = \"not a table\"", "ui = []", ""] {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("config.toml");
        fs::write(&source, text).unwrap();
        let output = prepare(dir.path(), None, Some(&source), "/owned.sock", Some(80));
        assert_eq!(output, source);
        assert_eq!(fs::read_to_string(&source).unwrap(), text);
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1);
    }
}

fn replaces_without_following_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let victim = dir.path().join("personal.toml");
    fs::write(&victim, "[ui]\nsidebar_width = 18\n").unwrap();
    let output = prepare(dir.path(), None, None, "/owned.sock", Some(160));
    fs::remove_file(&output).unwrap();
    std::os::unix::fs::symlink(&victim, &output).unwrap();
    let again = prepare(dir.path(), None, None, "/owned.sock", Some(160));
    assert_eq!(again, output);
    assert_eq!(
        fs::read_to_string(&victim).unwrap(),
        "[ui]\nsidebar_width = 18\n"
    );
    let written = fs::read_to_string(&output).unwrap();
    assert!(
        written.contains("sidebar_start_collapsed = false"),
        "{written}"
    );
    assert!(!output.symlink_metadata().unwrap().file_type().is_symlink());
}
