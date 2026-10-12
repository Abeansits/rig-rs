//! Pre-send process check. Paste waits until the pane is still the launched agent.
//!
//! The check and the later paste are not one atomic step. A process can
//! still change in that gap. That check-then-paste window matches the
//! TypeScript sender.

use std::sync::Arc;

use crewmux_core::Seat;
use crewmux_mux::{PaneInfo, PaneRef};
use crewmux_store::PaneLineage;

use crate::error::Error;
use crate::state::App;

const SHELLS: &[&str] = &[
    "bash", "zsh", "sh", "fish", "nu", "dash", "ksh", "tcsh", "csh",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    PidMismatch,
    BareShell,
    Backgrounded,
    SessionMismatch,
    ResumeUnproven,
}

impl Fault {
    const fn code(self) -> &'static str {
        match self {
            Self::PidMismatch => "pane_pid_mismatch",
            Self::BareShell => "bare_shell",
            Self::Backgrounded => "agent_backgrounded",
            Self::SessionMismatch => "session_id_mismatch",
            Self::ResumeUnproven => "resume_unproven",
        }
    }

    fn message(self) -> String {
        let detail = match self {
            Self::PidMismatch => "the pane process is not the one recorded at launch",
            Self::BareShell => "the pane is at a bare shell",
            Self::Backgrounded => "the pane foreground is not the agent",
            Self::SessionMismatch => {
                "the live process does not prove the session recorded at launch"
            }
            Self::ResumeUnproven => "exact resume cannot be proven",
        };
        format!("lineage:{}: {detail}. No text was sent.", self.code())
    }
}

/// Remember the pane process after a successful launch.
///
/// # Errors
///
/// Returns [`Error`] when the pane is missing or the row cannot be stored.
pub(crate) async fn record(app: &App, seat: &Seat, pane: &PaneRef) -> Result<(), Error> {
    let panes = app.mux.list_panes().await?;
    let live = panes
        .iter()
        .find(|info| same_pane(info, pane))
        .ok_or_else(|| Fault::ResumeUnproven.message())
        .map_err(Error::Message)?;
    let pid = live
        .pid
        .ok_or_else(|| Error::Message(Fault::PidMismatch.message()))?;
    let command = live.current_cmd.clone();
    let known_start = live.started_ticks;
    let from_argv = live.argv.as_deref().and_then(token_in_argv);
    let session_id = read_claude_id(app, seat).await.or(from_argv);
    let started_ticks = match known_start {
        Some(ticks) => Some(ticks),
        None => read_start(pid).await,
    };
    let row = PaneLineage {
        seat_id: seat.id.to_string(),
        pid,
        command,
        session_id,
        runtime: seat.runtime.as_str().to_owned(),
        started_ticks,
    };
    let store = Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || {
        store.write(move |conn| crewmux_store::lineage_put(conn, &row))
    })
    .await
    .map_err(crate::state::join_err)?
    .map_err(Error::from)
}

/// Refuse before paste when the pane is no longer the launched process.
///
/// Claude accepts a version-number process title (`2.1.289`) as well as
/// `claude` and `node`. Node's `process.title` overwrites the command line, and
/// `exec` keeps the pid and the start time, so those two do not prove which
/// session is running. A session or resume token in argv must be the one stored
/// at launch (a token the record lacks is a refusal). With no token, an open
/// file has to name that id, or the send is refused. Claude Code 2.1.286 keeps
/// `--session-id` in argv, so the native binary is covered by the token check.
/// A version title that erases the token is refused. An in-app resume that
/// leaves argv and open files unchanged is only caught after the send. A shell
/// in the live argv is a bare shell even when tmux still reports the command
/// recorded at launch. Stub seats use this same record.
///
/// # Errors
///
/// Returns [`Error::Message`] with a `lineage:` code. No paste has run yet.
pub(crate) async fn ensure(app: &App, seat: &Seat, pane: &PaneRef) -> Result<(), Error> {
    let seat_id = seat.id.to_string();
    let store = Arc::clone(&app.store);
    let recorded = tokio::task::spawn_blocking(move || {
        store.read(move |conn| crewmux_store::lineage_get(conn, &seat_id))
    })
    .await
    .map_err(crate::state::join_err)?
    .map_err(Error::from)?;
    let panes = app.mux.list_panes().await?;
    let live = panes.iter().find(|info| same_pane(info, pane));
    let snapshot = live.map(|info| (info.pid, info.argv.clone(), info.started_ticks));
    let (argv, live_started) = match snapshot {
        Some((pid, recorded_argv, known_start)) => {
            let argv = match recorded_argv {
                Some(argv) => Some(argv),
                None => match pid {
                    Some(pid) => read_cmdline(pid).await,
                    None => None,
                },
            };
            let started = match known_start {
                Some(ticks) => Some(ticks),
                None => match pid {
                    Some(pid) => read_start(pid).await,
                    None => None,
                },
            };
            (argv, started)
        }
        None => (None, None),
    };
    let holds_session = held_session(recorded.as_ref(), argv.as_deref(), live).await;
    if let Some(fault) = judge(
        recorded.as_ref(),
        live,
        argv.as_deref(),
        live_started,
        holds_session,
    ) {
        return Err(Error::Message(fault.message()));
    }
    Ok(())
}

async fn held_session(
    recorded: Option<&PaneLineage>,
    argv: Option<&str>,
    live: Option<&PaneInfo>,
) -> bool {
    let Some(expected) = recorded.and_then(|row| row.session_id.as_deref()) else {
        return false;
    };
    if token_in_argv(argv.unwrap_or("")).is_some() {
        return false;
    }
    let Some(pid) = live.and_then(|info| info.pid) else {
        return false;
    };
    crate::lineage_held::process_holds_session(pid, expected).await
}

fn judge(
    recorded: Option<&PaneLineage>,
    live: Option<&PaneInfo>,
    argv: Option<&str>,
    live_started: Option<u64>,
    holds_session: bool,
) -> Option<Fault> {
    let Some(recorded) = recorded else {
        return Some(Fault::ResumeUnproven);
    };
    let Some(live) = live else {
        return Some(Fault::PidMismatch);
    };
    let Some(pid) = live.pid else {
        return Some(Fault::PidMismatch);
    };
    if pid != recorded.pid {
        return Some(Fault::PidMismatch);
    }
    if let (Some(saved), Some(live_started)) = (recorded.started_ticks, live_started)
        && saved != live_started
    {
        return Some(Fault::PidMismatch);
    }
    if is_shell(&live.current_cmd) || argv_is_shell(argv) {
        return Some(Fault::BareShell);
    }
    if !same_foreground(&recorded.runtime, &recorded.command, &live.current_cmd) {
        return Some(Fault::Backgrounded);
    }
    session_fault(recorded, argv, holds_session)
}

/// `exec` keeps the pid and can leave tmux reporting the launch command.
///
/// The live argv is the process image. A shell there is a bare shell even
/// when `pane_current_command` still names the agent.
fn argv_is_shell(argv: Option<&str>) -> bool {
    argv.and_then(|text| text.split_whitespace().next())
        .is_some_and(|program| !program.is_empty() && is_shell(program))
}

fn session_fault(recorded: &PaneLineage, argv: Option<&str>, holds_session: bool) -> Option<Fault> {
    if recorded.runtime == "claude-code" && recorded.session_id.is_none() {
        return Some(Fault::ResumeUnproven);
    }
    if let Some(token) = argv.and_then(token_in_argv) {
        return match recorded.session_id.as_deref() {
            Some(expected) if expected == token => None,
            _ => Some(Fault::SessionMismatch),
        };
    }
    recorded.session_id.as_ref()?;
    if holds_session {
        None
    } else {
        Some(Fault::SessionMismatch)
    }
}

/// `node` and a version title are the same Claude process.
///
/// Real Claude sets `process.title` to its version, which replaces the
/// command name tmux reports.
fn same_foreground(runtime: &str, recorded: &str, live: &str) -> bool {
    let recorded = command_base(recorded);
    let live = command_base(live);
    if recorded == live {
        return true;
    }
    runtime == "claude-code" && claude_process(recorded) && claude_process(live)
}

fn claude_process(name: &str) -> bool {
    matches!(name, "claude" | "node" | "nodejs") || version_process(name)
}

/// `2.1.289`, the shape of Claude's `process.title`.
fn version_process(name: &str) -> bool {
    let mut parts = name.split('.');
    let (Some(major), Some(minor), Some(patch)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    all_digits(major) && all_digits(minor) && patch_starts_with_digit(patch)
}

fn all_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

fn patch_starts_with_digit(text: &str) -> bool {
    matches!(text.chars().next(), Some(ch) if ch.is_ascii_digit())
}

fn same_pane(info: &PaneInfo, pane: &PaneRef) -> bool {
    match &pane.pane_id {
        Some(id) if info.pane_id == *id => true,
        _ => info.session == pane.session,
    }
}

fn command_base(command: &str) -> &str {
    let name = command.rsplit(['/', '\\']).next().unwrap_or(command);
    name.strip_prefix('-').unwrap_or(name)
}

fn is_shell(command: &str) -> bool {
    SHELLS.contains(&command_base(command))
}

#[cfg(test)]
fn session_in_argv(argv: &str, expected: &str) -> bool {
    token_in_argv(argv).as_deref() == Some(expected)
}

fn token_in_argv(argv: &str) -> Option<String> {
    let tokens: Vec<&str> = argv.split_whitespace().collect();
    flag_value(&tokens, "--session-id")
        .or_else(|| flag_value(&tokens, "--resume"))
        .or_else(|| resume_word(&tokens))
        .map(str::to_owned)
}

fn flag_value<'a>(tokens: &[&'a str], flag: &str) -> Option<&'a str> {
    let equals = format!("{flag}=");
    for (index, token) in tokens.iter().enumerate() {
        if let Some(value) = token.strip_prefix(equals.as_str())
            && !value.is_empty()
            && !value.starts_with('-')
        {
            return Some(value);
        }
        if *token == flag {
            let next = tokens.get(index.saturating_add(1)).copied()?;
            if !next.starts_with('-') {
                return Some(next);
            }
        }
    }
    None
}

fn resume_word<'a>(tokens: &[&'a str]) -> Option<&'a str> {
    let index = tokens.iter().position(|token| *token == "resume")?;
    let mut cursor = index.saturating_add(1);
    while let Some(token) = tokens.get(cursor) {
        if token.starts_with('-') {
            cursor = cursor.saturating_add(1);
            continue;
        }
        return Some(*token);
    }
    None
}

async fn read_claude_id(app: &App, seat: &Seat) -> Option<String> {
    let path = app
        .home
        .join("seats")
        .join(seat.id.to_string())
        .join("claude-session-id");
    let text = tokio::fs::read_to_string(path).await.ok()?;
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_owned())
    }
}

async fn read_start(pid: u32) -> Option<u64> {
    tokio::task::spawn_blocking(move || crewmux_mux::process_start_ticks(pid))
        .await
        .ok()
        .flatten()
}

async fn read_cmdline(pid: u32) -> Option<String> {
    let path = format!("/proc/{pid}/cmdline");
    if let Some(text) = tokio::task::spawn_blocking(move || std::fs::read(path))
        .await
        .ok()
        .and_then(std::result::Result::ok)
        .map(|bytes| String::from_utf8_lossy(&bytes).replace('\0', " "))
    {
        let text = text.trim();
        if !text.is_empty() {
            return Some(text.to_owned());
        }
    }
    read_ps_args(pid).await
}

async fn read_ps_args(pid: u32) -> Option<String> {
    let output = tokio::process::Command::new("ps")
        .args(["-ww", "-p", &pid.to_string(), "-o", "args="])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crewmux_mux::{FakeMux, Geometry, Multiplexer, PaneInfo};
    use crewmux_store::PaneLineage;

    use super::{Fault, judge, session_in_argv};
    use crate::deliver::{Outbound, deliver};
    use crate::flow::{harness_with, two_stub};
    use crate::launch::bring_up;
    use crate::state::App;

    fn home() -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let path = std::env::temp_dir().join(format!("rig-lineage-{nanos}"));
        std::fs::create_dir_all(&path).expect("home");
        path
    }

    async fn launched() -> (App, Arc<FakeMux>) {
        let (app, mux) = harness_with(None, &home(), None).await;
        bring_up(&app, &two_stub("/tmp")).await.expect("up");
        (app, mux)
    }

    fn outbound() -> Outbound {
        Outbound {
            from: "operator@host".to_owned(),
            targets: vec!["one@pair".to_owned()],
            rig: None,
            all: false,
            body: "hello".to_owned(),
            interact: false,
            reason: None,
            verify: false,
            track_direct: true,
        }
    }

    fn pastes(calls: &[Vec<String>]) -> usize {
        calls
            .iter()
            .filter(|argv| argv.iter().any(|arg| arg == "paste-buffer"))
            .count()
    }

    async fn one_pane(mux: &FakeMux) -> crewmux_mux::PaneInfo {
        mux.list_panes()
            .await
            .expect("panes")
            .into_iter()
            .find(|pane| pane.session.contains("one"))
            .expect("one")
    }

    async fn refuse(mux: &FakeMux, app: &App, code: &str) {
        let before = pastes(&mux.recorded().await);
        let err = deliver(app, outbound()).await.expect_err("refused");
        assert!(err.to_string().contains(code), "{err}");
        assert_eq!(pastes(&mux.recorded().await), before);
    }

    #[test]
    fn argv_session_forms() {
        assert!(session_in_argv("claude --session-id abc", "abc"));
        assert!(session_in_argv("claude --resume=abc", "abc"));
        assert!(session_in_argv("codex resume abc", "abc"));
        assert!(!session_in_argv("claude --session-id other", "abc"));
    }

    #[tokio::test]
    async fn pid_mismatch_refuses_before_paste() {
        let (app, mux) = launched().await;
        let pane = one_pane(&mux).await;
        mux.set_pane_process(&pane.session, Some(99), &pane.current_cmd, None)
            .await;
        refuse(&mux, &app, "lineage:pane_pid_mismatch").await;
    }

    #[tokio::test]
    async fn bare_shell_refuses_before_paste() {
        let (app, mux) = launched().await;
        let pane = one_pane(&mux).await;
        mux.set_pane_process(&pane.session, pane.pid, "-zsh", None)
            .await;
        refuse(&mux, &app, "lineage:bare_shell").await;
    }

    #[tokio::test]
    async fn backgrounded_agent_refuses_before_paste() {
        let (app, mux) = launched().await;
        let pane = one_pane(&mux).await;
        mux.set_pane_process(&pane.session, pane.pid, "sleep", None)
            .await;
        refuse(&mux, &app, "lineage:agent_backgrounded").await;
    }

    #[tokio::test]
    async fn session_id_mismatch_refuses_before_paste() {
        let (app, mux) = launched().await;
        let pane = one_pane(&mux).await;
        plant_session(&app, "expected-session").await;
        mux.set_pane_process(
            &pane.session,
            pane.pid,
            &pane.current_cmd,
            Some("rig-stub-agent --session-id other-session"),
        )
        .await;
        refuse(&mux, &app, "lineage:session_id_mismatch").await;
    }

    #[tokio::test]
    async fn same_process_without_a_session_token_or_open_file_refuses() {
        let (app, mux) = launched().await;
        let pane = one_pane(&mux).await;
        plant_session(&app, "expected-session").await;
        mux.set_pane_process(
            &pane.session,
            pane.pid,
            &pane.current_cmd,
            Some("rig-stub-agent"),
        )
        .await;
        let seat = crate::query::find_address(&app, "one@pair")
            .await
            .expect("seat");
        let pref = crewmux_mux::PaneRef {
            session: pane.session,
            window: Some(0),
            pane: Some(0),
            pane_id: Some(pane.pane_id),
        };
        let err = super::ensure(&app, &seat, &pref)
            .await
            .expect_err("no session file");
        assert!(
            err.to_string().contains("lineage:session_id_mismatch"),
            "{err}"
        );
    }

    #[test]
    fn version_title_holding_the_recorded_session_is_the_same_agent() {
        let after = sample_row("node", Some("sess"), 9);
        let before = sample_row("2.1.289", Some("sess"), 9);
        let live = sample_pane("2.1.289", 9);
        assert_eq!(
            judge(Some(&after), Some(&live), Some("2.1.289"), Some(9), true),
            None
        );
        assert_eq!(
            judge(Some(&before), Some(&live), Some("2.1.289"), Some(9), true),
            None
        );
    }

    #[test]
    fn same_start_time_without_a_session_file_is_not_proof() {
        let row = sample_row("2.1.289", Some("sess"), 9);
        let live = sample_pane("2.1.289", 9);
        assert_eq!(
            judge(Some(&row), Some(&live), Some("2.1.289"), Some(9), false),
            Some(Fault::SessionMismatch)
        );
    }

    #[test]
    fn same_start_time_still_refuses_a_different_session_token() {
        let row = sample_row("node", Some("sess"), 9);
        let live = sample_pane("node", 9);
        assert_eq!(
            judge(
                Some(&row),
                Some(&live),
                Some("node --session-id other"),
                Some(9),
                true
            ),
            Some(Fault::SessionMismatch)
        );
    }

    #[test]
    fn shell_argv_is_a_bare_shell_when_tmux_still_names_the_agent() {
        let row = PaneLineage {
            seat_id: "s".to_owned(),
            pid: 1,
            command: "rig-stub-agent".to_owned(),
            session_id: None,
            runtime: "stub".to_owned(),
            started_ticks: Some(9),
        };
        let live = sample_pane("rig-stub-agent", 9);
        assert_eq!(
            judge(Some(&row), Some(&live), Some("bash"), Some(9), false),
            Some(Fault::BareShell)
        );
        assert_eq!(
            judge(
                Some(&row),
                Some(&live),
                Some("python3 /tmp/stay.py"),
                Some(9),
                false
            ),
            None
        );
    }

    #[test]
    fn a_reused_pid_with_a_new_start_time_is_a_mismatch() {
        let row = sample_row("node", Some("sess"), 9);
        let live = sample_pane("node", 10);
        assert_eq!(
            judge(
                Some(&row),
                Some(&live),
                Some("node --session-id sess"),
                Some(10),
                true
            ),
            Some(Fault::PidMismatch)
        );
    }

    fn sample_row(command: &str, session: Option<&str>, started: u64) -> PaneLineage {
        PaneLineage {
            seat_id: "s".to_owned(),
            pid: 1,
            command: command.to_owned(),
            session_id: session.map(str::to_owned),
            runtime: "claude-code".to_owned(),
            started_ticks: Some(started),
        }
    }

    fn sample_pane(command: &str, started: u64) -> PaneInfo {
        PaneInfo {
            pane_id: "%1".to_owned(),
            window_id: "@1".to_owned(),
            session: "seat".to_owned(),
            pid: Some(1),
            started_ticks: Some(started),
            current_cmd: command.to_owned(),
            argv: None,
            dead: false,
            dead_status: None,
            size: Geometry { cols: 80, rows: 24 },
            in_mode: false,
        }
    }

    #[tokio::test]
    async fn claude_without_a_session_cannot_prove_resume() {
        let (app, mux) = launched().await;
        let store = std::sync::Arc::clone(&app.store);
        tokio::task::spawn_blocking(move || {
            store.write(|conn| {
                conn.execute(
                    "UPDATE pane_lineage SET runtime = 'claude-code', session_id = NULL",
                    [],
                )?;
                Ok(())
            })
        })
        .await
        .expect("join")
        .expect("runtime");
        refuse(&mux, &app, "lineage:resume_unproven").await;
    }

    #[tokio::test]
    async fn codex_resume_token_the_record_lacks_refuses() {
        let (app, mux) = launched().await;
        let pane = one_pane(&mux).await;
        let store = std::sync::Arc::clone(&app.store);
        tokio::task::spawn_blocking(move || {
            store.write(|conn| {
                conn.execute(
                    "UPDATE pane_lineage SET runtime = 'codex', session_id = NULL",
                    [],
                )?;
                Ok(())
            })
        })
        .await
        .expect("join")
        .expect("runtime");
        mux.set_pane_process(
            &pane.session,
            pane.pid,
            &pane.current_cmd,
            Some("codex resume 22222222-2222-4222-8222-222222222222"),
        )
        .await;
        refuse(&mux, &app, "lineage:session_id_mismatch").await;
    }

    #[tokio::test]
    async fn startup_paste_refuses_when_the_pane_is_a_shell() {
        let (app, mux) = launched().await;
        let pane = one_pane(&mux).await;
        mux.set_pane_process(&pane.session, pane.pid, "bash", None)
            .await;
        let seat = crate::query::find_address(&app, "one@pair")
            .await
            .expect("seat");
        let dir = app.home.join("startup");
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("one.txt"), "boot-line\n").expect("startup");
        let before = pastes(&mux.recorded().await);
        let err = crate::direct::record_startup(&app, std::slice::from_ref(&seat))
            .await
            .expect_err("refused");
        assert!(err.to_string().contains("lineage:bare_shell"), "{err}");
        assert_eq!(pastes(&mux.recorded().await), before);
    }

    #[tokio::test]
    async fn restore_resend_refuses_when_the_pane_is_a_shell() {
        let (app, mux) = launched().await;
        app.set_direct_phase(Some("before")).await;
        deliver(&app, outbound()).await.expect("recorded");
        app.set_direct_phase(None).await;
        let pane = one_pane(&mux).await;
        mux.set_pane_process(&pane.session, pane.pid, "bash", None)
            .await;
        let before = pastes(&mux.recorded().await);
        crate::restore::restore_rig(&app, "pair", None, crewmux_types::LaunchPosture::Auto)
            .await
            .expect("restore");
        assert_eq!(pastes(&mux.recorded().await), before);
    }

    async fn plant_session(app: &App, session: &str) {
        let session = session.to_owned();
        let store = std::sync::Arc::clone(&app.store);
        tokio::task::spawn_blocking(move || {
            store.write(move |conn| {
                conn.execute(
                    "UPDATE pane_lineage SET session_id = ?1",
                    rusqlite::params![session],
                )?;
                Ok(())
            })
        })
        .await
        .expect("join")
        .expect("session");
    }
}
