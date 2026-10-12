//! CLI goldens for queue JSON, send refusal, and ask guidance.

mod common;

use std::time::Duration;

use common::{Env, stdout, two_stub};
use serde_json::{Value, json};

fn json_out(command: &mut std::process::Command) -> (i32, Value, String) {
    let (code, out, err) = stdout(command);
    let value = serde_json::from_str(out.trim()).unwrap_or(Value::Null);
    (code, value, err)
}

fn freeze(value: &Value) -> Value {
    match value {
        Value::Number(number) if number.as_i64().is_some_and(|raw| raw > 1_000_000_000_000) => {
            json!(0)
        }
        Value::Array(items) => Value::Array(items.iter().map(freeze).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, child)| (key.clone(), freeze(child)))
                .collect(),
        ),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => value.clone(),
    }
}

#[test]
fn queue_list_and_show_json() {
    let env = Env::new();
    let (code, _, err) = json_out(env.rig().args([
        "queue",
        "create",
        "--id",
        "baton-1",
        "--source",
        "alice@rig",
        "--destination",
        "bob@rig",
        "--body",
        "hello",
    ]));
    assert_eq!(code, 0, "{err}");
    let (code, listed, err) = json_out(env.rig().args(["queue", "list", "--json"]));
    assert_eq!(code, 0, "{err}");
    insta::assert_snapshot!(
        "queue_list",
        serde_json::to_string_pretty(&freeze(&listed)).expect("json")
    );
    let (code, shown, err) = json_out(env.rig().args(["queue", "show", "baton-1", "--json"]));
    assert_eq!(code, 0, "{err}");
    insta::assert_snapshot!(
        "queue_show",
        serde_json::to_string_pretty(&freeze(&shown)).expect("json")
    );
}

#[test]
fn send_refuses_an_interactive_gate() {
    let env = Env::new();
    let spec = env.write_spec("gate.yaml", &two_stub(&env.home));
    let (code, _, err) = json_out(
        env.rig()
            .env("RIG_STUB_SCRIPT", "print_gate=trust_gate")
            .args(["up", spec.to_str().expect("utf8"), "--json"]),
    );
    assert_eq!(code, 0, "{err}");
    let (code, _out, err) = stdout(env.rig().args(["send", "--to", "one@pair", "hello"]));
    assert_eq!(code, 1, "{err}");
    insta::assert_snapshot!("send_refusal", err.trim());
}

#[test]
fn ask_says_transcripts_are_disabled() {
    let env = Env::new();
    let (code, out, err) = stdout(
        env.rig()
            .env("RIG_TRANSCRIPTS", "0")
            .args(["ask", "--seat", "one", "hello"]),
    );
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("disabled"), "{out}");
    insta::assert_snapshot!("ask_disabled", out.trim());
}

#[test]
fn ask_says_when_the_seat_has_no_transcript() {
    let env = Env::new();
    let (code, out, err) = stdout(env.rig().args(["ask", "--seat", "one", "hello"]));
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("transcript"), "{out}");
    insta::assert_snapshot!("ask_missing", out.trim());
}

#[test]
fn ask_does_not_mix_seat_and_session_files() {
    let env = Env::new();
    let seat = env.rig_home.join("transcripts/pair/one");
    std::fs::create_dir_all(&seat).expect("seat");
    std::fs::write(seat.join("1.txt"), "seat-only-token\n").expect("txt");
    let sessions = env.rig_home.join("sessions");
    std::fs::create_dir_all(&sessions).expect("sessions");
    std::fs::write(sessions.join("tok.jsonl"), "session-only-token\n").expect("jsonl");
    let (code, seat_hit, err) = stdout(env.rig().args(["ask", "--seat", "one", "seat-only-token"]));
    assert_eq!(code, 0, "{err}");
    assert!(seat_hit.contains("seat-only-token"), "{seat_hit}");
    let (code, leaked, err) =
        stdout(
            env.rig()
                .args(["ask", "--seat", "one", "session-only-token"]),
        );
    assert_eq!(code, 0, "{err}");
    assert!(!leaked.contains("session-only-token"), "{leaked}");
    assert!(leaked.contains("No matching evidence"), "{leaked}");
    let (code, session_hit, err) =
        stdout(
            env.rig()
                .args(["ask", "--session", "tok", "session-only-token"]),
        );
    assert_eq!(code, 0, "{err}");
    assert!(session_hit.contains("session-only-token"), "{session_hit}");
    let (code, not_seat, err) =
        stdout(
            env.rig()
                .args(["ask", "--session", "tok", "seat-only-token"]),
        );
    assert_eq!(code, 0, "{err}");
    assert!(
        not_seat.contains("does not fall back") || not_seat.contains("No matching content"),
        "{not_seat}"
    );
    assert!(!not_seat.contains("seat-only-token"), "{not_seat}");
    let (code, unscoped, err) = stdout(env.rig().args(["ask", "hello"]));
    assert_eq!(code, 0, "{err}");
    assert!(
        unscoped.contains("whole-rig grep is not used"),
        "{unscoped}"
    );
}

#[test]
fn send_partial_exits_3() {
    let env = Env::new();
    let spec = env.write_spec("two.yaml", &two_stub(&env.home));
    let (code, _, err) = json_out(
        env.rig()
            .args(["up", spec.to_str().expect("utf8"), "--json"]),
    );
    assert_eq!(code, 0, "{err}");
    let (code, body, err) =
        json_out(
            env.rig()
                .args(["send", "--to", "one@pair,missing", "hello", "--json"]),
        );
    assert_eq!(code, 3, "{err} {body}");
    assert_eq!(body.get("status").and_then(Value::as_str), Some("partial"));
}

#[test]
fn tui_client_resumes_canonical_events() {
    let env = Env::new();
    let (code, _, err) = json_out(env.rig().args([
        "queue",
        "create",
        "--id",
        "evt-1",
        "--source",
        "alice@rig",
        "--destination",
        "bob@rig",
        "--body",
        "first",
    ]));
    assert_eq!(code, 0, "{err}");
    let socket = crewmux_api::daemon_socket(&env.rig_home);
    let first = tui_events(&socket, None, 1).expect("sse");
    let last = first.first().expect("event").id;
    assert!(last >= 1);
    let (code, _, err) = json_out(env.rig().args([
        "queue",
        "create",
        "--id",
        "evt-2",
        "--source",
        "alice@rig",
        "--destination",
        "bob@rig",
        "--body",
        "second",
    ]));
    assert_eq!(code, 0, "{err}");
    let more = tui_events(&socket, Some(last), 8).expect("resume");
    assert!(
        more.iter().any(|event| event.id > last),
        "missed events: {more:?}"
    );
    assert!(
        more.iter()
            .all(|event| event.kind != crewmux_api::StreamKind::Gap),
        "backlog should still hold both creates: {more:?}"
    );
}

fn tui_events(
    socket: &std::path::Path,
    last: Option<u64>,
    limit: usize,
) -> Result<Vec<crewmux_api::StreamEvent>, crewmux_tui::Error> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(crewmux_tui::Error::Runtime)?;
    runtime.block_on(crewmux_tui::read_events(
        socket,
        last,
        limit,
        Duration::from_secs(5),
    ))
}
