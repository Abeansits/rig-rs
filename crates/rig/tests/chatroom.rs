//! `rig chatroom` against a real daemon and `FakeMux`.
#![expect(
    clippy::expect_used,
    reason = "setup failures should abort the integration test"
)]

mod common;

use std::io::Read;
use std::process::Stdio;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use common::{Env, stdout, two_stub};
use serde_json::Value;

fn up(env: &Env) {
    assert!(env.rig_home.starts_with(env.root.path()));
    let spec = env.write_spec("two-stub.yaml", &two_stub(&env.home));
    let (code, out, err) = stdout(
        env.rig()
            .args(["up", spec.to_str().expect("utf8"), "--json"]),
    );
    assert_eq!(code, 0, "{err} {out}");
}

fn redact(text: &str) -> String {
    let value: Value = serde_json::from_str(text.trim()).unwrap_or(Value::Null);
    let mut raw = text.trim().to_owned();
    let mut ids = Vec::new();
    let mut times = Vec::new();
    collect(&value, &mut ids, &mut times);
    for id in ids {
        raw = raw.replace(&id, "<id>");
    }
    for time in times {
        raw = raw.replace(&time, "<time>");
    }
    raw
}

fn collect(value: &Value, ids: &mut Vec<String>, times: &mut Vec<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect(item, ids, times);
            }
        }
        Value::Object(map) => {
            for (key, child) in map {
                match (key.as_str(), child.as_str()) {
                    ("id" | "rigId", Some(text)) => ids.push(text.to_owned()),
                    ("createdAt", Some(text)) => times.push(text.to_owned()),
                    _ => collect(child, ids, times),
                }
            }
        }
        Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null => {}
    }
}

fn seat_proof(env: &Env) -> (String, String) {
    let path = env.rig_home.join("state/rig.db");
    let conn = rusqlite::Connection::open(&path).expect("db");
    conn.busy_timeout(Duration::from_secs(2)).expect("busy");
    conn.query_row(
        "SELECT seat_id, token FROM seat_tokens LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .expect("seat token")
}

#[test]
fn chatroom_send_json_is_operator() {
    let env = Env::new();
    up(&env);
    let (code, out, err) = stdout(
        env.rig()
            .args(["chatroom", "send", "pair", "hello", "--json"]),
    );
    assert_eq!(code, 0, "{err} {out}");
    let value = serde_json::from_str::<Value>(out.trim()).expect("json");
    assert_eq!(value["sender"], "operator");
    assert_eq!(value["body"], "hello");
    assert_eq!(value["kind"], "message");
    assert!(value["topic"].is_null());
    let raw = out.trim();
    let order = [
        "\"id\"",
        "\"rigId\"",
        "\"sender\"",
        "\"kind\"",
        "\"body\"",
        "\"topic\"",
        "\"createdAt\"",
    ];
    let mut rest = raw;
    for key in order {
        let found = rest.find(key).expect(key);
        let next = found.saturating_add(key.len());
        rest = rest.get(next..).unwrap_or("");
    }
    insta::assert_snapshot!(redact(&out));
}

#[test]
fn chatroom_history_prints_chronological() {
    let env = Env::new();
    up(&env);
    let (code, _, err) = stdout(env.rig().args(["chatroom", "send", "pair", "first"]));
    assert_eq!(code, 0, "{err}");
    let (code, _, err) = stdout(env.rig().args(["chatroom", "send", "pair", "second"]));
    assert_eq!(code, 0, "{err}");
    let (code, out, err) = stdout(env.rig().args(["chatroom", "history", "pair"]));
    assert_eq!(code, 0, "{err} {out}");
    insta::assert_snapshot!(out.trim());
}

#[test]
fn chatroom_history_json() {
    let env = Env::new();
    up(&env);
    let (code, _, err) = stdout(env.rig().args(["chatroom", "send", "pair", "only"]));
    assert_eq!(code, 0, "{err}");
    let (code, out, err) = stdout(env.rig().args(["chatroom", "history", "pair", "--json"]));
    assert_eq!(code, 0, "{err} {out}");
    let rows = serde_json::from_str::<Value>(out.trim()).expect("json");
    assert_eq!(rows.as_array().expect("array").len(), 1);
    assert_eq!(rows[0]["body"], "only");
    insta::assert_snapshot!(redact(&out));
}

#[test]
fn chatroom_topic_creates_marker() {
    let env = Env::new();
    up(&env);
    let (code, out, err) = stdout(env.rig().args(["chatroom", "topic", "pair", "deploy"]));
    assert_eq!(code, 0, "{err} {out}");
    insta::assert_snapshot!(out.trim());
    let (code, json, err) = stdout(env.rig().args([
        "chatroom", "topic", "pair", "standup", "--body", "daily", "--json",
    ]));
    assert_eq!(code, 0, "{err} {json}");
    let value = serde_json::from_str::<Value>(json.trim()).expect("json");
    assert_eq!(value["kind"], "topic");
    assert_eq!(value["topic"], "standup");
    insta::assert_snapshot!("topic_json", redact(&json));
}

#[test]
fn chatroom_history_sender_since_and_after() {
    let env = Env::new();
    up(&env);
    let (seat, token) = seat_proof(&env);
    let (code, _, err) = stdout(env.rig().args(["chatroom", "send", "pair", "from-op"]));
    assert_eq!(code, 0, "{err}");
    let (code, seated, err) = stdout(
        env.rig()
            .env("RIG_SEAT_ID", &seat)
            .env("RIG_SEAT_TOKEN", &token)
            .args(["chatroom", "send", "pair", "from-seat", "--json"]),
    );
    assert_eq!(code, 0, "{err} {seated}");
    assert_eq!(
        serde_json::from_str::<Value>(seated.trim()).expect("json")["sender"].as_str(),
        Some(seat.as_str())
    );
    let (code, filtered, err) = stdout(
        env.rig()
            .args(["chatroom", "history", "pair", "--sender", "operator"]),
    );
    assert_eq!(code, 0, "{err} {filtered}");
    assert!(filtered.contains("[operator] from-op"), "{filtered}");
    assert!(!filtered.contains("from-seat"), "{filtered}");
    let (code, json, err) = stdout(env.rig().args([
        "chatroom", "history", "pair", "--sender", "operator", "--json",
    ]));
    assert_eq!(code, 0, "{err} {json}");
    let rows = serde_json::from_str::<Value>(json.trim()).expect("json");
    assert_eq!(rows.as_array().expect("array").len(), 1);
    assert_eq!(rows[0]["body"], "from-op");
    insta::assert_snapshot!("sender_json", redact(&json));
    let (code, future, err) = stdout(env.rig().args([
        "chatroom",
        "history",
        "pair",
        "--since",
        "2099-01-01T00:00:00Z",
    ]));
    assert_eq!(code, 0, "{err} {future}");
    assert_eq!(future.trim(), "No messages.");
    let (code, past, err) = stdout(env.rig().args([
        "chatroom",
        "history",
        "pair",
        "--since",
        "2020-01-01T00:00:00Z",
    ]));
    assert_eq!(code, 0, "{err}");
    assert!(
        past.contains("from-op") && past.contains("from-seat"),
        "{past}"
    );
    let first = rows[0]["id"].as_str().expect("id");
    let (code, after, err) = stdout(
        env.rig()
            .args(["chatroom", "history", "pair", "--after", first]),
    );
    assert_eq!(code, 0, "{err} {after}");
    assert!(after.contains("from-seat"), "{after}");
    assert!(!after.contains("from-op"), "{after}");
}

#[test]
fn chatroom_clear_prints_deleted_count() {
    let env = Env::new();
    up(&env);
    let (code, _, err) = stdout(env.rig().args(["chatroom", "send", "pair", "gone"]));
    assert_eq!(code, 0, "{err}");
    let (code, out, err) = stdout(env.rig().args(["chatroom", "clear", "pair"]));
    assert_eq!(code, 0, "{err} {out}");
    insta::assert_snapshot!(out.trim());
    let (code, _, err) = stdout(env.rig().args(["chatroom", "send", "pair", "also-gone"]));
    assert_eq!(code, 0, "{err}");
    let (code, json, err) = stdout(env.rig().args(["chatroom", "clear", "pair", "--json"]));
    assert_eq!(code, 0, "{err} {json}");
    insta::assert_snapshot!("clear_json", json.trim());
}

#[test]
fn chatroom_wrong_token_is_rejected() {
    let env = Env::new();
    up(&env);
    let (seat, _) = seat_proof(&env);
    let (code, out, err) = stdout(
        env.rig()
            .env("RIG_SEAT_ID", &seat)
            .env("RIG_SEAT_TOKEN", "not-the-token")
            .args(["chatroom", "send", "pair", "nope"]),
    );
    assert_eq!(code, 1, "{out} {err}");
    assert!(err.contains("unknown sender"), "{err}");
}

#[test]
fn chatroom_watch_prints_and_exits_on_sigint() {
    let env = Env::new();
    up(&env);
    let (code, _, err) = stdout(env.rig().args(["chatroom", "send", "pair", "streamed"]));
    assert_eq!(code, 0, "{err}");
    let mut child = env
        .rig()
        .args(["chatroom", "watch", "pair"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("watch");
    let mut stdout_pipe = child.stdout.take().expect("stdout");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut raw = Vec::new();
        let mut buf = [0_u8; 256];
        loop {
            match stdout_pipe.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if let Some(chunk) = buf.get(..n) {
                        raw.extend_from_slice(chunk);
                    }
                    drop(tx.send(String::from_utf8_lossy(&raw).into_owned()));
                }
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut seen = String::new();
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(text) => {
                seen = text;
                if seen.contains("[operator] streamed") {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    let raw = i32::try_from(child.id()).expect("pid");
    let pid = rustix::process::Pid::from_raw(raw).expect("pid");
    rustix::process::kill_process(pid, rustix::process::Signal::INT).expect("int");
    let status = child.wait().expect("wait");
    assert!(seen.contains("[operator] streamed"), "{seen}");
    assert!(status.success(), "{status} {seen}");
}
