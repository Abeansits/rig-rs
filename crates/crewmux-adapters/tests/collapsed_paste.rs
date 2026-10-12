//! Collapsed Claude paste on a real pane.
//!
//! `+N` in `[Pasted text #N +X lines]` is the newline count. A 96-line prompt
//! with no trailing newline is `+95`. A file of 29 newlines, ending in a
//! newline, is `+29`. `+96` is the split-line count and must not be accepted.
//! The fake mirrors that collapse: the raw body is recorded to the session
//! jsonl and is not drawn, so a full-text composer match cannot hide a wrong
//! count.

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
    use std::time::Duration;

    use crewmux_adapters::{
        AgentAdapter, BudgetOverrides, ClaudeAdapter, LaunchBudget, LaunchOpts, Readiness, SeatIo,
        SubmitMode,
    };
    use crewmux_mux::{Multiplexer, TmuxConfig, TmuxMux};
    use crewmux_types::SeatId;

    static SOCKETS: AtomicU64 = AtomicU64::new(0);

    #[tokio::test]
    async fn collapsed_placeholder_is_the_newline_count() {
        let n = SOCKETS.fetch_add(1, Ordering::Relaxed);
        let socket = format!("rig-rs-{}-collapse{n}", std::process::id());
        let mux = TmuxMux::connect(TmuxConfig {
            binary: PathBuf::from("tmux"),
            socket,
            pause_after: false,
        })
        .await
        .expect("tmux");
        let root = scratch();
        let log = root.join("collapse.log");
        let script = install(&root);
        let cwd = root.join("work");
        let home = root.join("home");
        std::fs::create_dir_all(&cwd).expect("cwd");
        std::fs::create_dir_all(&home).expect("home");
        let shared: Arc<dyn Multiplexer> = Arc::new(mux);
        let adapter = ClaudeAdapter::new(Arc::clone(&shared), root.join("seats"), home.clone())
            .with_submit_wait(Duration::from_secs(3));
        let session = format!("collapse__{}", std::process::id());
        let binding = crewmux_adapters::binding_for(SeatId::new(), cwd.clone(), &session);
        let opts = launch_opts(&session, &script, &home, &cwd, &log);
        drop(adapter.launch(&binding, opts).await.expect("launch"));
        assert_eq!(
            adapter.check_ready(&binding).await.expect("ready"),
            Readiness::Ready
        );
        let io = adapter.io_for(&binding).await.expect("io");
        let prompt = (0..96)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let file = format!(
            "{}\n",
            (0..29)
                .map(|index| format!("row {index}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        deliver(&io, &prompt, &log).await;
        deliver(&io, &file, &log).await;
        drop(shared.kill_session(&session).await);
        let text = std::fs::read_to_string(&log).expect("log");
        assert!(
            text.contains("[Pasted text #1 +95 lines]"),
            "96 lines are +95 newlines\n{text}"
        );
        assert!(
            text.contains("[Pasted text #2 +29 lines]"),
            "a trailing newline is part of the count\n{text}"
        );
        assert!(
            !text.contains("+96"),
            "split-line +96 is not Claude's count\n{text}"
        );
        assert!(!text.contains("line 0"), "the raw prompt was drawn\n{text}");
        assert!(!text.contains("row 0"), "the raw file was drawn\n{text}");
        drop(std::fs::remove_dir_all(&root));
    }

    async fn deliver(io: &impl SeatIo, text: &str, log: &Path) {
        if let Err(err) = io.submit(text, SubmitMode::Enter).await {
            let dumped = std::fs::read_to_string(log).unwrap_or_default();
            let screen = io
                .snapshot()
                .await
                .map(|shot| shot.text)
                .unwrap_or_default();
            panic!("{err}\nscreen:\n{screen}\nlog:\n{dumped}");
        }
    }

    fn launch_opts(
        session: &str,
        script: &Path,
        home: &Path,
        cwd: &Path,
        log: &Path,
    ) -> LaunchOpts {
        let mut opts = LaunchOpts {
            session_name: session.to_owned(),
            env: std::collections::BTreeMap::new(),
            resume: None,
            fork: false,
            auto_trust: false,
            budget: Some(LaunchBudget::with_overrides(BudgetOverrides {
                first_screen: Some(Duration::from_secs(3)),
                ready_after_last_gate: Some(Duration::from_secs(3)),
                inconclusive_stable: Some(Duration::from_secs(3)),
                overall: Some(Duration::from_secs(8)),
                poll: Some(Duration::from_millis(50)),
            })),
            claude_bin: Some(script.to_path_buf()),
        };
        opts.env
            .insert("HOME".to_owned(), home.display().to_string());
        opts.env
            .insert("PYTHONUNBUFFERED".to_owned(), "1".to_owned());
        opts.env.insert("FAKE_COLLAPSE".to_owned(), "1".to_owned());
        opts.env
            .insert("FAKE_COLLAPSE_LOG".to_owned(), log.display().to_string());
        // macOS getcwd resolves /var to /private/var. The adapter slugs the cwd
        // it was given, so the fixture must write the jsonl under that path.
        opts.env
            .insert("RIG_CWD".to_owned(), cwd.display().to_string());
        opts
    }

    fn install(root: &Path) -> PathBuf {
        let script = root.join("claude");
        std::fs::write(&script, FAKE_COLLAPSE).expect("script");
        let mut perms = std::fs::metadata(&script).expect("meta").permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(0o755);
        }
        std::fs::set_permissions(&script, perms).expect("chmod");
        script
    }

    fn scratch() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "rig-collapse-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos())
        ));
        drop(std::fs::remove_dir_all(&root));
        std::fs::create_dir_all(&root).expect("scratch");
        root
    }

    const FAKE_COLLAPSE: &str = r#"#!/usr/bin/env python3
import json, os, select, sys, termios, threading, time

args = sys.argv[1:]
sid = args[args.index("--session-id") + 1]
home = os.environ["HOME"]
cwd = os.environ.get("RIG_CWD", os.getcwd())
slug = "".join(ch if ch.isascii() and ch.isalnum() else "-" for ch in cwd)
path = os.path.join(home, ".claude", "projects", slug, sid + ".jsonl")
os.makedirs(os.path.dirname(path), exist_ok=True)
log_path = os.environ.get("FAKE_COLLAPSE_LOG", "")
collapse = os.environ.get("FAKE_COLLAPSE") == "1"
START = b"\x1b[200~"
END = b"\x1b[201~"
lock = threading.Lock()
state = {"mode": "ready", "line": "❯ "}
seq = 0
pending = b""
raw = b""
tick = 0

def log(text):
    if not log_path:
        return
    with open(log_path, "a", encoding="utf-8") as handle:
        handle.write(text)
        if not text.endswith("\n"):
            handle.write("\n")

def paint():
    rule = "─" * 80
    if state["mode"] == "paste":
        foot = "  paste again to expand\n"
    else:
        foot = "  ? for shortcuts %s\n" % tick
    frame = rule + "\n" + state["line"] + "\n" + rule + "\n" + foot
    sys.stdout.write("\x1b[?2004h\x1b[2J\x1b[3J\x1b[H" + frame)
    sys.stdout.flush()
    return frame

def ticker():
    global tick
    while True:
        time.sleep(0.01)
        with lock:
            if state["mode"] != "ready":
                continue
            tick += 1
            paint()

def record(text):
    with open(path, "a", encoding="utf-8") as handle:
        handle.write(json.dumps({"type": "user", "message": {"role": "user", "content": text}}) + "\n")

def accept(body):
    global seq, raw
    text = body.decode("utf-8", "replace")
    newlines = text.count("\n")
    seq += 1
    with lock:
        if collapse and newlines > 0:
            state["mode"] = "paste"
            state["line"] = "❯ [Pasted text #%s +%s lines]" % (seq, newlines)
        else:
            state["mode"] = "paste"
            state["line"] = "❯ "
        frame = paint()
    log(frame)
    record(text)
    raw = b""

def enter():
    with lock:
        state["mode"] = "ready"
        state["line"] = "❯ "
        paint()

def parse():
    global pending, raw
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
            if raw:
                chunk = raw
                raw = b""
                accept(chunk)
            enter()
            continue
        cut = 0
        while cut < len(pending) and pending[cut] not in (0x1b, 0x0d, 0x0a):
            cut += 1
        raw += pending[:cut]
        pending = pending[cut:]

def raw_input():
    fd = sys.stdin.fileno()
    attr = termios.tcgetattr(fd)
    attr[0] = attr[0] & ~(termios.ICRNL | termios.INLCR | termios.IXON)
    attr[3] = attr[3] & ~(termios.ECHO | termios.ICANON | termios.IEXTEN | termios.ISIG)
    termios.tcsetattr(fd, termios.TCSANOW, attr)

raw_input()
with lock:
    paint()
log("READY")
threading.Thread(target=ticker, daemon=True).start()
fd = sys.stdin.fileno()
while True:
    wait = 0.05 if raw else None
    ready, _, _ = select.select([fd], [], [], wait)
    if not ready:
        if raw and b"\n" in raw:
            chunk = raw
            raw = b""
            accept(chunk)
        continue
    data = os.read(fd, 65536)
    if data == b"":
        break
    pending += data
    parse()
"#;
}
