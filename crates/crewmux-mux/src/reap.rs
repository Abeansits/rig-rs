//! Nudge tmux to reap pane exits.
//!
//! Ubuntu's tmux is built with libutempter. Closing a pane pty from that build
//! can swallow `SIGCHLD`, so the server leaves a zombie: `pane_dead` is set,
//! `pane_dead_status` stays empty, and the `pane-died` hook never runs.
//! Upstream tmux sends itself `SIGCHLD` after utempter calls. This task does
//! that from outside, which is enough for `waitpid` to record the status and
//! fire the hook.

use std::sync::Arc;
use std::time::Duration;

use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::error::MuxError;
use crate::exec::TmuxExec;

/// Until `cancel`, periodically signal the tmux server so it reaps children.
pub(crate) async fn nudge(exec: Arc<dyn TmuxExec>, cancel: CancellationToken) {
    let Ok(pid) = server_pid(exec.as_ref()).await else {
        tracing::debug!("tmux reap nudge skipped");
        return;
    };
    let pid = pid.to_string();
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(Duration::from_millis(15)) => {}
        }
        if cancel.is_cancelled() {
            return;
        }
        match signal_chld(&pid).await {
            Ok(true) => {}
            Ok(false) | Err(_) => return,
        }
    }
}

async fn server_pid(exec: &dyn TmuxExec) -> Result<u32, MuxError> {
    let out = exec
        .run(&[
            "display-message".to_owned(),
            "-p".to_owned(),
            "#{pid}".to_owned(),
        ])
        .await?;
    let text = out.stdout.trim();
    let pid = text
        .parse::<u32>()
        .map_err(|_| MuxError::Control(format!("tmux server pid missing: {text}")))?;
    if pid == 0 {
        return Err(MuxError::Control("tmux server pid missing".to_owned()));
    }
    Ok(pid)
}

/// `true` when the signal was delivered.
async fn signal_chld(pid: &str) -> Result<bool, std::io::Error> {
    let status = Command::new("kill")
        .arg("-s")
        .arg("CHLD")
        .arg(pid)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await?;
    Ok(status.success())
}
