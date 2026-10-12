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

use crewmux_adapters::codex::LaunchBudget;
use crewmux_adapters::{CodexAdapter, LaunchOpts, NodeBinding};
use crewmux_mux::{FakeMux, Multiplexer};
use crewmux_types::{LaunchPosture, SeatId};

pub(crate) const LOGGED_IN: &str = "#!/bin/sh\nprintf '%s\\n' 'Logged in using ChatGPT'\nexit 0\n";
pub(crate) const NOT_LOGGED_IN: &str = "#!/bin/sh\nprintf '%s\\n' 'Not logged in'\nexit 1\n";
pub(crate) const MISSING_HOME: &str =
    "#!/bin/sh\nprintf '%s\\n' 'Error finding codex home config.toml: does not exist'\nexit 1\n";
pub(crate) const UNRECOGNISED: &str = "#!/bin/sh\nprintf '%s\\n' 'status unavailable'\nexit 2\n";
pub(crate) const HANGING: &str = "#!/bin/sh\nexec sleep 60\n";

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
        thread_id_after_submit: Duration::from_millis(400),
        preflight: Duration::from_secs(2),
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
        auto_trust: false,
        budget: None,
        claude_bin: None,
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
        lines.push(user_line(text));
    }
    fs::write(&path, lines.join("\n") + "\n").expect("rollout");
    path
}

pub(crate) fn user_line(text: &str) -> String {
    serde_json::json!({
        "type": "response_item",
        "payload": {
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": text}]
        }
    })
    .to_string()
}

pub(crate) fn append_line(path: &Path, line: &str) {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(path)
        .expect("rollout");
    writeln!(file, "{line}").expect("append");
}

pub(crate) fn append_user(path: &Path, text: &str) {
    append_line(path, &user_line(text));
}

pub(crate) fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/screens/codex")
}

pub(crate) fn calls_include(calls: &[Vec<String>], token: &str) -> bool {
    calls.iter().any(|call| call.iter().any(|arg| arg == token))
}
