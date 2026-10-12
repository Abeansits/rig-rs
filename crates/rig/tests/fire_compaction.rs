//! `fire_compaction` runs the real `rig hook` binary with recorded payloads.
#![expect(
    clippy::expect_used,
    reason = "a missing binary or fixture should fail the test"
)]

use std::path::PathBuf;

use assert_cmd::Command;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/hooks/claude/2.1.286/hooks.jsonl")
}

fn fire_compaction(
    home: &std::path::Path,
    event: &str,
    payload: &str,
) -> assert_cmd::assert::Assert {
    Command::cargo_bin("rig")
        .expect("rig")
        .arg("hook")
        .arg(event)
        .env("RIG_HOME", home)
        .env("RIG_SEAT_KEY", "seat-key")
        .env("RIG_RUN_ID", "run-1")
        .write_stdin(payload)
        .assert()
}

#[test]
fn fire_compaction_recorded_payload() {
    let home = tempfile::tempdir().expect("home");
    let body = std::fs::read_to_string(fixture()).expect("fixture");
    let payload = body
        .lines()
        .find_map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).ok()?;
            if value.get("argv_event").and_then(serde_json::Value::as_str) == Some("PreCompact") {
                value.get("input").cloned()
            } else {
                None
            }
        })
        .expect("precompact");
    let text = serde_json::to_string(&payload).expect("encode");
    fire_compaction(home.path(), "PreCompact", &text)
        .success()
        .stdout("");
    let spool = home.path().join("seats/seat-key/hooks.spool.jsonl");
    let stored = std::fs::read_to_string(&spool).expect("spool");
    assert!(stored.contains("PreCompact"));
    assert!(stored.contains("b5461a7f-0588-427a-b7a2-31a9bb2c5b99"));
    assert!(stored.contains("run-1"));
    assert!(stored.ends_with('\n'));
}

#[test]
fn malformed_hook_exits_0() {
    let home = tempfile::tempdir().expect("home");
    fire_compaction(home.path(), "PreCompact", "{")
        .success()
        .stdout("")
        .stderr(predicates::str::contains("malformed stdin"));
    assert!(
        !home
            .path()
            .join("seats/seat-key/hooks.spool.jsonl")
            .exists()
    );
}
