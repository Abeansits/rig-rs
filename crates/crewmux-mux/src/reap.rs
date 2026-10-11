//! Nudge tmux to reap pane exits.
//!
//! Ubuntu's tmux is built with libutempter. Closing a pane pty from that build
//! can swallow `SIGCHLD`, so the server leaves a zombie: `pane_dead` is set,
//! `pane_dead_status` stays empty, and the `pane-died` hook never runs.
//! Upstream tmux sends itself `SIGCHLD` after utempter calls. This task does
//! that from outside, which is enough for `waitpid` to record the status and
//! fire the hook.
//!
//! The nudge runs only on Linux. Other platforms already reap, and spawning
//! `kill` on every tick pushed a macOS trial past the 100ms death budget.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::exec::TmuxExec;

/// Until `cancel`, periodically signal the tmux server so it reaps children.
pub(crate) async fn nudge(exec: Arc<dyn TmuxExec>, cancel: CancellationToken) {
    #[cfg(target_os = "linux")]
    linux_nudge(exec, cancel).await;
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (exec, cancel);
    }
}

#[cfg(target_os = "linux")]
async fn linux_nudge(exec: Arc<dyn TmuxExec>, cancel: CancellationToken) {
    use std::time::Duration;

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

#[cfg(target_os = "linux")]
async fn server_pid(exec: &dyn TmuxExec) -> Result<u32, crate::error::MuxError> {
    use crate::error::MuxError;

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
#[cfg(target_os = "linux")]
async fn signal_chld(pid: &str) -> Result<bool, std::io::Error> {
    let status = tokio::process::Command::new("kill")
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
