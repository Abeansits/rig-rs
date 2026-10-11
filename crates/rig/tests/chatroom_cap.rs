//! At-cap chat bodies must survive `rig chatroom watch` in both output modes.
#![expect(
    clippy::expect_used,
    reason = "setup failures should abort the integration test"
)]

mod common;

use std::io::Read;
use std::process::{Child, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
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

#[test]
fn watch_prints_at_cap_quotes_and_nuls() {
    assert_watch(false);
}

#[test]
fn watch_json_prints_at_cap_quotes_and_nuls() {
    assert_watch(true);
}

fn assert_watch(json: bool) {
    let env = Env::new();
    up(&env);
    let mut args = vec!["chatroom", "watch", "pair"];
    if json {
        args.push("--json");
    }
    let mut child = env
        .rig()
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("watch");
    let rx = spawn_reader(&mut child);
    let (code, out, err) = stdout(
        env.rig()
            .args(["chatroom", "send", "pair", "ready", "--json"]),
    );
    assert_eq!(code, 0, "{err} {out}");
    let sent = serde_json::from_str::<Value>(out.trim()).expect("send json");
    let rig_id = sent.get("rigId").and_then(Value::as_str).expect("rig id");
    let ready = wait_for(&rx, "ready");
    assert!(ready.contains("ready"), "len {}", ready.len());
    let cap = crewmux_api::MAX_CHAT_BODY;
    post_body(&env, rig_id, &"\"".repeat(cap));
    post_body(&env, rig_id, &"\0".repeat(cap));
    let (code, _, err) = stdout(env.rig().args(["chatroom", "send", "pair", "after-cap"]));
    assert_eq!(code, 0, "{err}");
    let seen = wait_for(&rx, "after-cap");
    assert!(
        child.try_wait().expect("poll").is_none(),
        "watch exited before the trailer, len {}",
        seen.len()
    );
    if json {
        assert_json_bodies(&seen, cap);
    } else {
        assert_human_bodies(&seen, cap);
    }
    let raw = i32::try_from(child.id()).expect("pid");
    let pid = rustix::process::Pid::from_raw(raw).expect("pid");
    rustix::process::kill_process(pid, rustix::process::Signal::INT).expect("int");
    let status = child.wait().expect("wait");
    let watch_err = read_stderr(&mut child);
    assert!(status.success(), "{status} {watch_err}");
    assert!(
        !watch_err.contains("oversized"),
        "dropped a legal frame: {watch_err}"
    );
}

fn assert_human_bodies(seen: &str, cap: usize) {
    let quotes = format!("[operator] {}", "\"".repeat(cap));
    assert!(
        seen.contains(&quotes),
        "quote body missing, len {}",
        seen.len()
    );
    assert_eq!(
        seen.matches("\\x00").count(),
        cap,
        "nul escapes {}, len {}",
        seen.matches("\\x00").count(),
        seen.len()
    );
    assert!(seen.contains("[operator] after-cap"), "len {}", seen.len());
}

fn assert_json_bodies(seen: &str, cap: usize) {
    let mut quotes = false;
    let mut nuls = false;
    let mut trailer = false;
    for line in seen.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let payload = value.get("payload").unwrap_or(&value);
        let Some(body) = payload.get("body").and_then(Value::as_str) else {
            continue;
        };
        if body.len() == cap && body.chars().all(|ch| ch == '"') {
            quotes = true;
        }
        if body.len() == cap && body.chars().all(|ch| ch == '\0') {
            nuls = true;
        }
        if body == "after-cap" {
            trailer = true;
        }
    }
    assert!(quotes, "json quote body missing, len {}", seen.len());
    assert!(nuls, "json nul body missing, len {}", seen.len());
    assert!(trailer, "json trailer missing, len {}", seen.len());
}

fn post_body(env: &Env, rig_id: &str, body: &str) {
    let payload = serde_json::to_vec(&serde_json::json!({"body": body})).expect("payload");
    let socket = crewmux_api::daemon_socket(&env.rig_home);
    let client = crewmux_api::Client::over_uds(socket).with_timeout(Duration::from_secs(10));
    let path = format!("/api/rigs/{rig_id}/chat/send");
    let response = client.request("POST", &path, Some(&payload)).expect("post");
    assert_eq!(response.status, 201, "len {}", response.body.len());
}

fn spawn_reader(child: &mut Child) -> Receiver<String> {
    let mut stdout_pipe = child.stdout.take().expect("stdout");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut raw = Vec::new();
        let mut buf = [0_u8; 8192];
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
    rx
}

fn wait_for(rx: &Receiver<String>, needle: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut seen = String::new();
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(text) => {
                seen = text;
                if seen.contains(needle) {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    seen
}

fn read_stderr(child: &mut Child) -> String {
    child
        .stderr
        .take()
        .map(|mut pipe| {
            let mut text = String::new();
            drop(pipe.read_to_string(&mut text));
            text
        })
        .unwrap_or_default()
}
