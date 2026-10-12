//! Pre-send process check. Paste waits until the pane is still the launched agent.

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
            Self::PidMismatch => "the pane pid does not match the pid recorded at launch",
            Self::BareShell => "the pane is at a bare shell",
            Self::Backgrounded => "the pane foreground is not the agent",
            Self::SessionMismatch => "the session-id in argv does not match",
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
    let from_argv = live.argv.as_deref().and_then(token_in_argv);
    let session_id = read_claude_id(app, seat).await.or(from_argv);
    let row = PaneLineage {
        seat_id: seat.id.to_string(),
        pid,
        command,
        session_id,
        runtime: seat.runtime.as_str().to_owned(),
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
    let argv = match live {
        Some(info) => match info.argv.clone() {
            Some(argv) => Some(argv),
            None => match info.pid {
                Some(pid) => read_cmdline(pid).await,
                None => None,
            },
        },
        None => None,
    };
    if let Some(fault) = judge(recorded.as_ref(), live, argv.as_deref()) {
        return Err(Error::Message(fault.message()));
    }
    Ok(())
}

fn judge(
    recorded: Option<&PaneLineage>,
    live: Option<&PaneInfo>,
    argv: Option<&str>,
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
    if is_shell(&live.current_cmd) {
        return Some(Fault::BareShell);
    }
    if command_base(&live.current_cmd) != command_base(&recorded.command) {
        return Some(Fault::Backgrounded);
    }
    if recorded.runtime == "claude-code" && recorded.session_id.is_none() {
        return Some(Fault::ResumeUnproven);
    }
    let expected = recorded.session_id.as_deref()?;
    let Some(argv) = argv.filter(|text| !text.is_empty()) else {
        return Some(Fault::ResumeUnproven);
    };
    if session_in_argv(argv, expected) {
        None
    } else {
        Some(Fault::SessionMismatch)
    }
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

fn session_in_argv(argv: &str, expected: &str) -> bool {
    let tokens: Vec<&str> = argv.split_whitespace().collect();
    for (index, token) in tokens.iter().enumerate() {
        if let Some(value) = token
            .strip_prefix("--session-id=")
            .or_else(|| token.strip_prefix("--resume="))
            && value == expected
        {
            return true;
        }
        if (*token == "--session-id" || *token == "--resume")
            && tokens.get(index.saturating_add(1)).copied() == Some(expected)
        {
            return true;
        }
    }
    if let Some(index) = tokens.iter().position(|token| *token == "resume") {
        let mut cursor = index.saturating_add(1);
        while let Some(token) = tokens.get(cursor) {
            if token.starts_with('-') {
                cursor = cursor.saturating_add(1);
                continue;
            }
            return *token == expected;
        }
    }
    false
}

fn token_in_argv(argv: &str) -> Option<String> {
    let tokens: Vec<&str> = argv.split_whitespace().collect();
    for (index, token) in tokens.iter().enumerate() {
        if let Some(value) = token
            .strip_prefix("--session-id=")
            .or_else(|| token.strip_prefix("--resume="))
            && !value.is_empty()
            && !value.starts_with('-')
        {
            return Some(value.to_owned());
        }
        if *token == "--session-id" || *token == "--resume" {
            let next = tokens.get(index.saturating_add(1)).copied()?;
            if !next.starts_with('-') {
                return Some(next.to_owned());
            }
        }
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

async fn read_cmdline(pid: u32) -> Option<String> {
    let path = format!("/proc/{pid}/cmdline");
    let bytes = tokio::task::spawn_blocking(move || std::fs::read(path))
        .await
        .ok()?
        .ok()?;
    let text = String::from_utf8_lossy(&bytes).replace('\0', " ");
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crewmux_mux::{FakeMux, Multiplexer};

    use super::session_in_argv;
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
    async fn missing_session_id_refuses_before_paste() {
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
        refuse(&mux, &app, "lineage:session_id_mismatch").await;
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
