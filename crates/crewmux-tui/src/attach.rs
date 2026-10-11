//! External attach. The TUI suspends, runs tmux or a launcher, then resumes.

use std::io;
use std::pin::Pin;
use std::process::ExitStatus;
use std::sync::Arc;

use crewmux_api::TmuxSession;

use crate::guard::TerminalGuard;
use crate::msg::{AttachOutcome, AttachTarget};

/// Argv after the `tmux` binary.
#[must_use]
pub fn tmux_argv(target: &AttachTarget) -> Option<Vec<String>> {
    match target {
        AttachTarget::Tmux {
            socket, session, ..
        } => Some(attach_argv(socket, session, "attach")),
        AttachTarget::SwitchClient { socket, session } => {
            Some(attach_argv(socket, session, "switch-client"))
        }
        AttachTarget::Launcher { .. } => None,
    }
}

fn attach_argv(socket: &str, session: &TmuxSession, verb: &str) -> Vec<String> {
    vec![
        "-L".to_owned(),
        socket.to_owned(),
        verb.to_owned(),
        "-t".to_owned(),
        format!("={session}"),
    ]
}

/// What the app asks a Herdr or Ghostty launcher to open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchRequest {
    /// Seat label.
    pub seat: crewmux_api::SeatKey,
    /// tmux socket name.
    pub socket: String,
    /// tmux session.
    pub session: TmuxSession,
}

/// Launcher failure. The message is shown with the tmux fallback.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct LaunchFail {
    /// Operator-facing reason.
    pub message: String,
}

/// Owned launcher callback supplied by the binary. The TUI does not link integrations.
pub type LaunchFn = Arc<
    dyn Fn(
            LaunchRequest,
        ) -> Pin<Box<dyn std::future::Future<Output = Result<(), LaunchFail>> + Send>>
        + Send
        + Sync,
>;

/// Suspend, run `target`, resume.
///
/// `launch` is used for [`AttachTarget::Launcher`]. A launch error falls back to
/// `tmux attach` and returns [`AttachOutcome::DetachedWithNotice`].
pub async fn perform(
    target: AttachTarget,
    launch: Option<LaunchFn>,
    guard: &mut dyn TerminalGuard,
    daemon: &std::path::Path,
) -> AttachOutcome {
    if let Err(err) = guard.suspend() {
        return AttachOutcome::Failed(err.to_string());
    }
    let outcome = execute(target, launch, daemon).await;
    if let Err(err) = guard.resume() {
        return match outcome {
            AttachOutcome::SessionGone => AttachOutcome::SessionGone,
            AttachOutcome::Detached
            | AttachOutcome::DetachedWithNotice(_)
            | AttachOutcome::Failed(_) => AttachOutcome::Failed(err.to_string()),
        };
    }
    outcome
}

async fn execute(
    target: AttachTarget,
    launch: Option<LaunchFn>,
    daemon: &std::path::Path,
) -> AttachOutcome {
    match &target {
        AttachTarget::Launcher {
            seat,
            socket,
            session,
            ..
        } => launch_or_fallback(launch, seat.clone(), socket.clone(), session.clone()).await,
        AttachTarget::Tmux { seat, .. } => {
            let name = seat.as_str().to_owned();
            let path = daemon.to_path_buf();
            let socket = socket_of(&target).to_owned();
            let session = session_of(&target).clone();
            let from_daemon = tokio::task::spawn_blocking(move || {
                crewmux_api::Client::over_uds(path).attach_argv(&name)
            })
            .await;
            if let Ok(Ok(argv)) = from_daemon
                && let Some((program, args)) = argv.split_first()
            {
                return run_program(program, args, &socket, &session).await;
            }
            match tmux_argv(&target) {
                Some(args) => finish_tmux(&args, &socket, &session).await,
                None => AttachOutcome::Failed("nothing to attach".to_owned()),
            }
        }
        AttachTarget::SwitchClient { .. } => match tmux_argv(&target) {
            Some(args) => finish_tmux(&args, socket_of(&target), session_of(&target)).await,
            None => AttachOutcome::Failed("nothing to attach".to_owned()),
        },
    }
}

async fn run_program(
    program: &str,
    args: &[String],
    socket: &str,
    session: &TmuxSession,
) -> AttachOutcome {
    let status = match tokio::process::Command::new(program)
        .args(args)
        .status()
        .await
    {
        Ok(status) => status,
        Err(err) => return AttachOutcome::Failed(err.to_string()),
    };
    match session_alive(socket, session).await {
        Ok(false) => AttachOutcome::SessionGone,
        Ok(true) if status.success() => AttachOutcome::Detached,
        Ok(true) => AttachOutcome::Failed(format!("tmux exited {status}")),
        Err(err) => AttachOutcome::Failed(err.to_string()),
    }
}

async fn launch_or_fallback(
    launch: Option<LaunchFn>,
    seat: crewmux_api::SeatKey,
    socket: String,
    session: TmuxSession,
) -> AttachOutcome {
    let Some(launch) = launch else {
        return fallback(&socket, &session, "no launcher is configured").await;
    };
    let request = LaunchRequest {
        seat,
        socket: socket.clone(),
        session: session.clone(),
    };
    match launch(request).await {
        Ok(()) => AttachOutcome::Detached,
        Err(err) => fallback(&socket, &session, &err.message).await,
    }
}

async fn fallback(socket: &str, session: &TmuxSession, reason: &str) -> AttachOutcome {
    let args = attach_argv(socket, session, "attach");
    let outcome = finish_tmux(&args, socket, session).await;
    match outcome {
        AttachOutcome::Detached => AttachOutcome::DetachedWithNotice(format!(
            "launcher failed ({reason}); attached with tmux"
        )),
        AttachOutcome::DetachedWithNotice(_)
        | AttachOutcome::Failed(_)
        | AttachOutcome::SessionGone => outcome,
    }
}

async fn finish_tmux(args: &[String], socket: &str, session: &TmuxSession) -> AttachOutcome {
    let status = match run_tmux(args).await {
        Ok(status) => status,
        Err(err) => return AttachOutcome::Failed(err.to_string()),
    };
    match session_alive(socket, session).await {
        Ok(false) => AttachOutcome::SessionGone,
        Ok(true) if status.success() => AttachOutcome::Detached,
        Ok(true) => AttachOutcome::Failed(format!("tmux exited {status}")),
        Err(err) => AttachOutcome::Failed(err.to_string()),
    }
}

async fn session_alive(socket: &str, session: &TmuxSession) -> io::Result<bool> {
    let args = vec![
        "-L".to_owned(),
        socket.to_owned(),
        "has-session".to_owned(),
        "-t".to_owned(),
        format!("={session}"),
    ];
    Ok(run_tmux(&args).await?.success())
}

async fn run_tmux(args: &[String]) -> io::Result<ExitStatus> {
    tokio::process::Command::new("tmux")
        .args(args)
        .status()
        .await
}

fn socket_of(target: &AttachTarget) -> &str {
    match target {
        AttachTarget::Tmux { socket, .. }
        | AttachTarget::SwitchClient { socket, .. }
        | AttachTarget::Launcher { socket, .. } => socket.as_str(),
    }
}

fn session_of(target: &AttachTarget) -> &TmuxSession {
    match target {
        AttachTarget::Tmux { session, .. }
        | AttachTarget::SwitchClient { session, .. }
        | AttachTarget::Launcher { session, .. } => session,
    }
}
