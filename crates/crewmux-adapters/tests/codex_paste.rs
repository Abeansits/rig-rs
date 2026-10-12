//! Collapsed Codex paste on a real pane.
//!
//! Codex 0.162 draws a paste over 1000 characters as `[Pasted Content N chars]`,
//! where N is the Unicode scalar count. The fake mux still splices the raw
//! body into the capture, so only this pane shows the placeholder.

#![cfg(feature = "tmux-it")]

#[expect(
    clippy::expect_used,
    clippy::panic,
    reason = "temp-dir setup failures should fail the test"
)]
mod tmux_it {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    use crewmux_adapters::{AgentAdapter, CodexAdapter, Readiness, SubmitMode};
    use crewmux_mux::{CaptureRange, Multiplexer, TmuxConfig, TmuxMux};

    use super::support::{binding, fast_budget, home_with, opts};

    static SOCKETS: AtomicU64 = AtomicU64::new(0);

    #[tokio::test]
    async fn paste_collapse_accepts_the_char_count() {
        let n = SOCKETS.fetch_add(1, Ordering::Relaxed);
        let socket = format!("rig-rs-{}-cxpaste{n}", std::process::id());
        let mux = TmuxMux::connect(TmuxConfig {
            binary: PathBuf::from("tmux"),
            socket,
            pause_after: false,
        })
        .await
        .expect("tmux");
        let root = scratch();
        let log = root.join("paste.log");
        let script = install(&root);
        let home = home_with(LOGGED_IN_STUB);
        let session = format!("cxpaste-{}", std::process::id());
        let shared: Arc<dyn Multiplexer> = Arc::new(mux);
        let mut budget = fast_budget();
        budget.poll = Duration::from_millis(50);
        budget.thread_id_after_submit = Duration::from_secs(4);
        budget.preflight = Duration::from_secs(2);
        let adapter =
            CodexAdapter::with_bin(Arc::clone(&shared), script.clone()).with_budget(budget);
        let binding = binding(&home.cwd, &session);
        let mut launch = opts(&home, &session);
        launch
            .env
            .insert("RIG_CODEX_BIN".to_owned(), script.display().to_string());
        launch
            .env
            .insert("FAKE_PASTE_COLLAPSE".to_owned(), "1".to_owned());
        launch
            .env
            .insert("FAKE_PASTE_LOG".to_owned(), log.display().to_string());
        launch
            .env
            .insert("RIG_CWD".to_owned(), home.cwd.display().to_string());
        launch
            .env
            .insert("PYTHONUNBUFFERED".to_owned(), "1".to_owned());
        adapter.launch(&binding, launch).await.expect("launch");
        wait_ready(&adapter, &binding, &shared).await;
        let body = "é".repeat(1001);
        let submitted = adapter.submit(&binding, &body, SubmitMode::Enter).await;
        drop(shared.kill_session(&session).await);
        let submitted = match submitted {
            Ok(receipt) => receipt,
            Err(err) => {
                let dumped = std::fs::read_to_string(&log).unwrap_or_default();
                panic!("{err}\nlog:\n{dumped}");
            }
        };
        assert_eq!(submitted.bytes, body.len());
        let dumped = std::fs::read_to_string(&log).expect("log");
        let placeholder = format!("[Pasted Content {} chars]", body.chars().count());
        assert!(
            dumped.contains(&placeholder),
            "char count, not byte count\n{dumped}"
        );
        assert!(!dumped.contains("éé"), "the raw paste was drawn\n{dumped}");
        drop(std::fs::remove_dir_all(&root));
    }

    async fn wait_ready(
        adapter: &CodexAdapter,
        binding: &crewmux_adapters::NodeBinding,
        mux: &Arc<dyn Multiplexer>,
    ) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if adapter.check_ready(binding).await.expect("ready") == Readiness::Ready {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let shot = mux
            .capture(&binding.pane, CaptureRange::Visible)
            .await
            .map(|shot| shot.text)
            .unwrap_or_default();
        panic!("composer never became ready\n{shot}");
    }

    fn install(root: &Path) -> PathBuf {
        let script = root.join("codex");
        std::fs::write(&script, FAKE_CODEX).expect("script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script).expect("meta").permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script, perms).expect("chmod");
        }
        script
    }

    fn scratch() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "rig-cxpaste-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos())
        ));
        drop(std::fs::remove_dir_all(&root));
        std::fs::create_dir_all(&root).expect("scratch");
        root
    }

    const LOGGED_IN_STUB: &str = "#!/bin/sh\nprintf '%s\\n' 'Logged in using ChatGPT'\nexit 0\n";

    const FAKE_CODEX: &str = r#"#!/usr/bin/env python3
import json, os, sys, termios, time

args = sys.argv[1:]
if "login" in args and "status" in args:
    sys.stdout.write("Logged in using ChatGPT\n")
    raise SystemExit(0)

home = os.environ.get("CODEX_HOME", "")
cwd = os.environ.get("RIG_CWD", os.getcwd())
sid = "thread-paste"
stamp = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
day = time.strftime("%Y/%m/%d", time.gmtime())
path = os.path.join(home, "sessions", day, "rollout-" + sid + ".jsonl")
os.makedirs(os.path.dirname(path), exist_ok=True)
log_path = os.environ.get("FAKE_PASTE_LOG", "")
collapse = os.environ.get("FAKE_PASTE_COLLAPSE") == "1"
START = b"\x1b[200~"
END = b"\x1b[201~"
state = {"line": "› "}
last = {"text": ""}
pending = b""

def log(text):
    if not log_path:
        return
    with open(log_path, "a", encoding="utf-8") as handle:
        handle.write(text)
        if not text.endswith("\n"):
            handle.write("\n")

def paint():
    frame = "╭──╮\n│ %s│\n╰──╯\n  ? for shortcuts\n" % state["line"]
    sys.stdout.write("\x1b[?2004h\x1b[2J\x1b[H" + frame)
    sys.stdout.flush()
    log(frame)

def record(text):
    row = {
        "type": "response_item",
        "payload": {
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": text}],
        },
    }
    with open(path, "a", encoding="utf-8") as handle:
        handle.write(json.dumps(row, ensure_ascii=False) + "\n")

def open_rollout():
    row = {
        "type": "session_meta",
        "timestamp": stamp,
        "payload": {"id": sid, "cwd": cwd, "timestamp": stamp},
    }
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(json.dumps(row) + "\n")

def accept(body):
    text = body.decode("utf-8", "replace")
    last["text"] = text
    count = len(text)
    if collapse and count > 1000:
        state["line"] = "› [Pasted Content %d chars] " % count
    else:
        state["line"] = "› " + text.replace("\n", " ") + " "
    paint()

def enter():
    if last["text"]:
        record(last["text"])
        last["text"] = ""
    state["line"] = "› "
    paint()

def parse():
    global pending
    while pending:
        if pending.startswith(START):
            end = pending.find(END)
            if end < 0:
                return
            body = pending[len(START):end]
            pending = pending[end + len(END):]
            accept(body)
            continue
        if pending.startswith(b"\x1b"):
            if START.startswith(pending):
                return
            pending = pending[1:]
            continue
        if pending[:1] in (b"\r", b"\n"):
            pending = pending[1:]
            enter()
            continue
        cut = 0
        while cut < len(pending) and pending[cut] not in (0x1b, 0x0d, 0x0a):
            cut += 1
        pending = pending[cut:]

def raw_input():
    fd = sys.stdin.fileno()
    attr = termios.tcgetattr(fd)
    attr[0] = attr[0] & ~(termios.ICRNL | termios.INLCR | termios.IXON)
    attr[3] = attr[3] & ~(termios.ECHO | termios.ICANON | termios.IEXTEN | termios.ISIG)
    termios.tcsetattr(fd, termios.TCSANOW, attr)

raw_input()
open_rollout()
paint()
log("READY")
fd = sys.stdin.fileno()
while True:
    data = os.read(fd, 65536)
    if data == b"":
        break
    pending += data
    parse()
"#;
}

#[cfg(feature = "tmux-it")]
#[path = "codex_support.rs"]
mod support;
