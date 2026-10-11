//! Per-seat `OpenCode` JSON is never the user config directory.

use std::os::unix::fs::PermissionsExt;

use crewmux_provider::{OpenCodePermission, OpenCodeSeatConfig};

fn sample() -> OpenCodeSeatConfig {
    OpenCodeSeatConfig {
        permission: OpenCodePermission {
            edit: "allow".to_owned(),
            bash: "ask".to_owned(),
            webfetch: "ask".to_owned(),
        },
    }
}

#[test]
fn writes_permission_json_per_seat() {
    let dir = tempfile::tempdir().unwrap();
    let seat = dir.path().join("seats").join("one");
    let path = sample().write(&seat).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["permission"]["edit"], "allow");
    assert_eq!(parsed["permission"]["bash"], "ask");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn refuses_user_opencode_directory() {
    let dir = tempfile::tempdir().unwrap();
    let banned = dir.path().join(".config").join("opencode");
    let error = sample().write(&banned).unwrap_err();
    assert!(!banned.join("opencode.json").exists());
    drop(error);
}
