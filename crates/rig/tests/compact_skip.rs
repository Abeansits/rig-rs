//! `rig compact --skip-map` exits 0 when `/compact` cannot be transcript-confirmed.
//!
//! The prep turn is confirmed from the Claude jsonl. The fake multiplexer echoes
//! the paste to `RIG_PASTE_SINK` so this test can append that turn. `/compact`
//! is not copied, and the product confirm path still has to accept `NotVerified`.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;

use common::{Env, stdout, two_stub};

fn ready(env: &Env, sink: &Path) -> Command {
    let mut command = env.rig();
    command
        .env("RIG_FAKE_CAPTURE", "claude-ready")
        .env("RIG_SUBMIT_WAIT_MS", "1500")
        .env("RIG_PASTE_SINK", sink);
    command
}

#[test]
fn skip_map_exits_zero() {
    let env = Env::new();
    let work = env.home.join("work");
    let _stub_spec = two_stub(&work);
    std::fs::create_dir_all(&work).expect("work");
    let sink = env.home.join("paste-sink");
    let spec = env.write_spec(
        "claude.yaml",
        &format!(
            "name: pair\nseats:\n  - name: claude\n    runtime: claude-code\n    cwd: {}\n",
            work.display()
        ),
    );
    let (code, up, err) =
        stdout(ready(&env, &sink).args(["up", spec.to_str().expect("utf8"), "--json"]));
    assert_eq!(code, 0, "{err} {up}");
    let up_json: Value = serde_json::from_str(up.trim()).unwrap_or(Value::Null);
    assert_eq!(
        up_json.get("status").and_then(Value::as_str),
        Some("ready"),
        "{up_json}"
    );
    let session = claude_session(&env.rig_home);
    assert!(!session.is_empty(), "claude session id was not stored");
    let transcript =
        crewmux_transcript::transcript_path(&env.home, &work, &session).expect("transcript");
    let stop = Arc::new(AtomicBool::new(false));
    let (errors_tx, errors_rx) = std::sync::mpsc::channel();
    let mirror = mirror_prep(sink, transcript, Arc::clone(&stop), errors_tx);
    let (code, out, err) = stdout(ready(&env, &env.home.join("paste-sink")).args([
        "compact",
        "claude",
        "--skip-map",
        "--json",
    ]));
    stop.store(true, Ordering::Relaxed);
    mirror.join().expect("mirror");
    if let Ok(err) = errors_rx.try_recv() {
        panic!("transcript mirror failed: {err}");
    }
    assert_eq!(code, 0, "{err} {out}");
    assert!(
        !err.contains("did not confirm"),
        "stderr still reports a missed transcript: {err}"
    );
    assert!(!out.contains("occupant_generation_unavailable"), "{out}");
    let value: Value = serde_json::from_str(out.trim()).expect("json");
    assert_eq!(
        value.get("compact_sent"),
        Some(&Value::Bool(true)),
        "{value}"
    );
}

fn claude_session(rig_home: &Path) -> String {
    let seats = rig_home.join("seats");
    let Ok(entries) = std::fs::read_dir(seats) else {
        return String::new();
    };
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        if let Ok(text) = std::fs::read_to_string(entry.path().join("claude-session-id")) {
            return text.trim().to_owned();
        }
    }
    String::new()
}

fn mirror_prep(
    sink: PathBuf,
    transcript: PathBuf,
    stop: Arc<AtomicBool>,
    errors: std::sync::mpsc::Sender<String>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut carried = 0usize;
        while !stop.load(Ordering::Relaxed) {
            if let Ok(bytes) = std::fs::read(&sink) {
                while let Some((text, next)) = one_record(&bytes, carried) {
                    carried = next;
                    if text.contains("OpenRig manual compaction was requested")
                        && let Err(err) = append_turn(&transcript, &text)
                    {
                        drop(errors.send(err));
                    }
                }
            }
            std::thread::park_timeout(Duration::from_millis(10));
        }
    })
}

fn one_record(bytes: &[u8], at: usize) -> Option<(String, usize)> {
    let rest = bytes.get(at..)?;
    let nl = rest.iter().position(|byte| *byte == b'\n')?;
    let len_bytes = rest.get(..nl)?;
    let len_text = std::str::from_utf8(len_bytes).ok()?;
    let len: usize = len_text.parse().ok()?;
    let start = at.saturating_add(nl).saturating_add(1);
    let end = start.saturating_add(len);
    if bytes.len() < end {
        return None;
    }
    let text = std::str::from_utf8(bytes.get(start..end)?).ok()?.to_owned();
    Some((text, end))
}

fn append_turn(path: &Path, text: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "transcript parent".to_owned())?;
    std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    let mut line = serde_json::json!({
        "type": "user",
        "message": {"role": "user", "content": text}
    })
    .to_string();
    line.push('\n');
    let mut body = std::fs::read(path).unwrap_or_default();
    body.extend(line.into_bytes());
    std::fs::write(path, body).map_err(|err| err.to_string())
}
