//! Shared Codex seat fixtures for the integration tests.
#![expect(
    dead_code,
    reason = "each integration binary compiles this helper and uses a different subset"
)]
#![expect(
    clippy::expect_used,
    reason = "a missing fixture or temp directory fails the test"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use crewmux_adapters::codex::LaunchBudget;
use crewmux_adapters::dialog::{DialogChoice, DialogDriver, DriveResult, PaneIo};
use crewmux_adapters::{CodexAdapter, Error, LaunchOpts, NodeBinding};
use crewmux_mux::{FakeMux, Key, Multiplexer};
use crewmux_types::{LaunchPosture, SeatId};
use tokio::sync::Mutex;

pub(crate) const LOGGED_IN: &str = "#!/bin/sh\nprintf '%s\\n' 'Logged in using ChatGPT'\nexit 0\n";
pub(crate) const NOT_LOGGED_IN: &str = "#!/bin/sh\nprintf '%s\\n' 'Not logged in'\nexit 1\n";
pub(crate) const MISSING_HOME: &str =
    "#!/bin/sh\nprintf '%s\\n' 'Error finding codex home config.toml: does not exist'\nexit 1\n";
pub(crate) const UNRECOGNISED: &str = "#!/bin/sh\nprintf '%s\\n' 'status unavailable'\nexit 2\n";

pub(crate) const READY: &str =
    "╭────────────────╮\n│ ›              │\n╰────────────────╯\n  ? for shortcuts\n";

pub(crate) const STAMP: &str = "2099-01-01T00:00:00Z";

pub(crate) struct Home {
    _root: tempfile::TempDir,
    pub(crate) user_dir: PathBuf,
    pub(crate) codex_dir: PathBuf,
    pub(crate) cwd: PathBuf,
    pub(crate) bin: PathBuf,
}

pub(crate) fn home_with(script: &str) -> Home {
    let root = tempfile::tempdir().expect("temp");
    let home = root.path().join("home");
    let codex_home = home.join(".codex");
    let cwd = root.path().join("work");
    fs::create_dir_all(&codex_home).expect("codex home");
    fs::create_dir_all(&cwd).expect("cwd");
    let bin = root.path().join("codex");
    fs::write(&bin, script).expect("stub");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).expect("mode");
    }
    Home {
        _root: root,
        user_dir: home,
        codex_dir: codex_home,
        cwd,
        bin,
    }
}

pub(crate) fn fast_budget() -> LaunchBudget {
    LaunchBudget {
        poll: Duration::from_millis(10),
        first_screen: Duration::from_millis(50),
        ready_after_last_gate: Duration::from_millis(50),
        inconclusive_stable: Duration::from_millis(50),
        overall: Duration::from_secs(2),
        thread_id_after_submit: Duration::from_millis(80),
    }
}

pub(crate) fn adapter(mux: &Arc<FakeMux>, bin: &Path) -> CodexAdapter {
    let owned = Arc::clone(mux);
    let dyn_mux: Arc<dyn Multiplexer> = owned;
    CodexAdapter::with_bin(dyn_mux, bin.to_path_buf()).with_budget(fast_budget())
}

pub(crate) fn binding(cwd: &Path, session: &str) -> NodeBinding {
    NodeBinding {
        seat: SeatId::new(),
        cwd: cwd.to_path_buf(),
        model: None,
        effort: None,
        posture: LaunchPosture::Auto,
        permission_mode: None,
        non_interruptive: true,
        pane: crewmux_mux::PaneRef::session(session),
        launch_generation: "1".to_owned(),
        runtime_opts: serde_json::Value::Null,
    }
}

pub(crate) fn opts(home: &Home, session: &str) -> LaunchOpts {
    let mut env = std::collections::BTreeMap::new();
    env.insert("HOME".to_owned(), home.user_dir.display().to_string());
    env.insert(
        "CODEX_HOME".to_owned(),
        home.codex_dir.display().to_string(),
    );
    env.insert("RIG_CODEX_BIN".to_owned(), home.bin.display().to_string());
    LaunchOpts {
        session_name: session.to_owned(),
        env,
        resume: None,
        fork: false,
    }
}

pub(crate) fn write_rollout(
    codex_home: &Path,
    id: &str,
    cwd: &Path,
    user: Option<&str>,
) -> PathBuf {
    let dir = codex_home.join("sessions/2099/01/01");
    fs::create_dir_all(&dir).expect("sessions");
    let path = dir.join(format!("rollout-{id}.jsonl"));
    let cwd = cwd.canonicalize().expect("cwd");
    let mut lines = vec![
        serde_json::json!({
            "type": "session_meta",
            "timestamp": STAMP,
            "payload": {"id": id, "cwd": cwd, "timestamp": STAMP}
        })
        .to_string(),
    ];
    if let Some(text) = user {
        lines.push(
            serde_json::json!({
                "type": "response_item",
                "payload": {"role": "user", "text": text}
            })
            .to_string(),
        );
    }
    fs::write(&path, lines.join("\n") + "\n").expect("rollout");
    path
}

pub(crate) fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

pub(crate) fn calls_include(calls: &[Vec<String>], token: &str) -> bool {
    calls.iter().any(|call| call.iter().any(|arg| arg == token))
}

#[derive(Clone, Copy)]
pub(crate) enum Motion {
    Follow,
    Stuck,
    Wrong,
}

pub(crate) struct DriveReport {
    pub(crate) result: DriveResult,
    pub(crate) keys: Vec<Key>,
    pub(crate) before_enter: Option<String>,
}

pub(crate) async fn drive_menu(
    title: &str,
    options: &[&str],
    choice: DialogChoice,
    motion: Motion,
) -> DriveReport {
    let io = Replay::new(title, options, motion);
    let result = DialogDriver::answer(&io, choice).await.expect("drive");
    io.finish(result)
}

struct Replay {
    inner: Mutex<ReplayInner>,
}

struct ReplayInner {
    title: String,
    options: Vec<String>,
    selected: usize,
    keys: Vec<Key>,
    before_enter: Option<String>,
    motion: Motion,
    closed: bool,
}

impl Replay {
    fn new(title: &str, options: &[&str], motion: Motion) -> Self {
        Self {
            inner: Mutex::new(ReplayInner {
                title: title.to_owned(),
                options: options.iter().map(|label| (*label).to_owned()).collect(),
                selected: 0,
                keys: Vec::new(),
                before_enter: None,
                motion,
                closed: false,
            }),
        }
    }

    fn finish(self, result: DriveResult) -> DriveReport {
        let inner = self.inner.into_inner();
        DriveReport {
            result,
            keys: inner.keys,
            before_enter: inner.before_enter,
        }
    }
}

#[async_trait]
impl PaneIo for Replay {
    async fn capture_text(&self) -> Result<String, Error> {
        let guard = self.inner.lock().await;
        Ok(render(&guard))
    }

    async fn send_keys(&self, keys: &[Key]) -> Result<(), Error> {
        let mut inner = self.inner.lock().await;
        for key in keys {
            inner.keys.push(key.clone());
            match key {
                Key::Down => nudge(&mut inner, true),
                Key::Up => nudge(&mut inner, false),
                Key::Enter => {
                    inner.before_enter = Some(render(&inner));
                    if matches!(inner.motion, Motion::Follow) {
                        inner.closed = true;
                    }
                }
                Key::Escape | Key::CtrlC | Key::Tab | Key::Literal(_) => {}
            }
        }
        Ok(())
    }
}

fn nudge(inner: &mut ReplayInner, down: bool) {
    match inner.motion {
        Motion::Stuck => {}
        Motion::Wrong => inner.selected = inner.options.len().saturating_sub(1),
        Motion::Follow if down => {
            if inner.selected + 1 < inner.options.len() {
                inner.selected += 1;
            }
        }
        Motion::Follow => inner.selected = inner.selected.saturating_sub(1),
    }
}

fn render(inner: &ReplayInner) -> String {
    if inner.closed {
        return READY.to_owned();
    }
    let mut lines = vec![inner.title.clone(), String::new()];
    for (index, label) in inner.options.iter().enumerate() {
        let marker = if index == inner.selected { '›' } else { ' ' };
        let number = index.saturating_add(1);
        lines.push(format!("{marker} {number}. {label}"));
    }
    lines.join("\n")
}
