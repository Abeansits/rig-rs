//! Snapshot and restore-check JSON.
#![expect(
    clippy::expect_used,
    reason = "setup failures abort the integration test"
)]

mod common;

use common::{Env, stdout, two_stub};
use serde_json::Value;

#[test]
fn restore_check_daemon_down_json() {
    let env = Env::new();
    let (code, out, err) = stdout(env.rig().args(["restore-check", "--json"]));
    assert_eq!(code, 0, "{err}");
    let value: Value = serde_json::from_str(out.trim()).expect("json");
    assert_eq!(
        value.pointer("/classes/unknown").and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        value
            .pointer("/remediations/0/execution_safe")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        value
            .pointer("/remediations/0/action")
            .and_then(Value::as_str),
        Some("start the daemon")
    );
    insta::assert_snapshot!(out.trim());
}

#[test]
fn restore_check_all_ready_dead_and_missing_resume() {
    let env = Env::new();
    let spec = env.write_spec("pair.yaml", &two_stub(&env.home));
    let (code, _, err) = stdout(env.rig().arg("up").arg(&spec).arg("--json"));
    assert_eq!(code, 0, "{err}");

    let (code, ready, err) = stdout(env.rig().args(["restore-check", "--json"]));
    assert_eq!(code, 0, "{err}");
    let ready: Value = serde_json::from_str(ready.trim()).expect("json");
    assert_eq!(
        ready.pointer("/classes/ready").and_then(Value::as_u64),
        Some(2)
    );
    assert!(class_keys(&ready));
    insta::assert_snapshot!("all_ready", serde_json::to_string_pretty(&ready).unwrap());

    let (code, _, err) = stdout(env.rig().args(["down", "--json"]));
    assert_eq!(code, 0, "{err}");
    let (code, dead, err) = stdout(env.rig().args(["restore-check", "--json"]));
    assert_eq!(code, 0, "{err}");
    let dead: Value = serde_json::from_str(dead.trim()).expect("json");
    assert_eq!(
        dead.pointer("/classes/not_ready").and_then(Value::as_u64),
        Some(2)
    );
    assert!(unsafe_remediations(&dead));
    insta::assert_snapshot!("dead_seat", serde_json::to_string_pretty(&dead).unwrap());

    let (code, _, err) = stdout(env.rig().arg("up").arg(&spec).arg("--json"));
    assert_eq!(code, 0, "{err}");
    clear_resume(&env, "two");
    let (code, missing, err) = stdout(env.rig().args(["restore-check", "--json"]));
    assert_eq!(code, 0, "{err}");
    let missing: Value = serde_json::from_str(missing.trim()).expect("json");
    assert_eq!(
        missing.pointer("/classes/ready").and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        missing
            .pointer("/classes/ready_with_caveats")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert!(unsafe_remediations(&missing));
    insta::assert_snapshot!(
        "missing_resume",
        serde_json::to_string_pretty(&missing).unwrap()
    );
}

#[test]
fn snapshot_list_and_show_json() {
    let env = Env::new();
    let spec = env.write_spec("pair.yaml", &two_stub(&env.home));
    let (code, _, err) = stdout(env.rig().arg("up").arg(&spec).arg("--json"));
    assert_eq!(code, 0, "{err}");
    let (code, created, err) = stdout(
        env.rig()
            .args(["snapshot", "create", "--rig", "pair", "--json"]),
    );
    assert_eq!(code, 0, "{err}");
    let created: Value = serde_json::from_str(created.trim()).expect("json");
    let id = created
        .get("id")
        .and_then(Value::as_str)
        .expect("id")
        .to_owned();
    assert_eq!(
        created.get("sha256").and_then(Value::as_str).map(str::len),
        Some(64)
    );
    let (code, listed, err) = stdout(
        env.rig()
            .args(["snapshot", "list", "--rig", "pair", "--json"]),
    );
    assert_eq!(code, 0, "{err}");
    let mut listed: Value = serde_json::from_str(listed.trim()).expect("json");
    assert_eq!(
        listed.pointer("/snapshots/0").and_then(Value::as_str),
        Some(id.as_str())
    );
    redact(&mut listed);
    insta::assert_snapshot!("list", serde_json::to_string_pretty(&listed).unwrap());
    let (code, shown, err) = stdout(
        env.rig()
            .args(["snapshot", "show", "--rig", "pair", "--id", &id, "--json"]),
    );
    assert_eq!(code, 0, "{err}");
    let mut shown: Value = serde_json::from_str(shown.trim()).expect("json");
    assert_eq!(shown.get("schema_version").and_then(Value::as_u64), Some(7));
    redact(&mut shown);
    insta::assert_snapshot!("show", serde_json::to_string_pretty(&shown).unwrap());
}

fn class_keys(value: &Value) -> bool {
    [
        "ready",
        "ready_with_caveats",
        "not_ready",
        "attention_required",
        "unknown",
    ]
    .iter()
    .all(|key| value.pointer(&format!("/classes/{key}")).is_some())
}

fn unsafe_remediations(value: &Value) -> bool {
    value
        .get("remediations")
        .and_then(Value::as_array)
        .is_some_and(|rows| {
            !rows.is_empty()
                && rows.iter().all(|row| {
                    row.get("execution_safe").and_then(Value::as_bool) == Some(false)
                        && row.get("action").and_then(Value::as_str).is_some()
                })
        })
}

fn clear_resume(env: &Env, seat: &str) {
    let path = env.rig_home.join("state/rig.db");
    let conn = rusqlite::Connection::open(&path).expect("db");
    conn.execute(
        "UPDATE seat_restore SET resume_value = NULL, resume_kind = NULL
         WHERE seat_id = (SELECT id FROM seats WHERE name = ?1)",
        [seat],
    )
    .expect("update");
}

fn redact(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(redact),
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if key == "created_at" {
                    *child = Value::from(0);
                } else if key == "sha256" {
                    *child = Value::from("<sha256>");
                } else if let Value::String(text) = child {
                    if is_ulid(text) {
                        *child = Value::from("<ulid>");
                    } else if text.starts_with("stub-") {
                        *child = Value::from("<resume>");
                    }
                } else {
                    redact(child);
                }
            }
        }
        Value::String(text) if is_ulid(text) => "<ulid>".clone_into(text),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn is_ulid(text: &str) -> bool {
    text.len() == 26
        && text
            .chars()
            .all(|ch| "0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(ch))
}
