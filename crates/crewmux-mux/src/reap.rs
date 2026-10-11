//! Nudge tmux to reap pane exits.
//!
//! Ubuntu's tmux is built with libutempter. Closing a pane pty from that build
//! can swallow `SIGCHLD`, so the server leaves a zombie: `pane_dead` is set,
//! `pane_dead_status` stays empty, and the `pane-died` hook never runs.
//! Upstream tmux sends itself `SIGCHLD` after utempter calls. On a rig socket
//! this task does that with `rustix` while a pane is dead and still has no
//! exit status.
//!
//! The nudge runs only on Linux. Other platforms already reap.

use std::sync::Arc;

use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::exec::TmuxExec;
use crate::types::MuxEvent;

/// Until `cancel`, signal the tmux server so it reaps children.
pub(crate) async fn nudge(
    exec: Arc<dyn TmuxExec>,
    socket: String,
    events: broadcast::Receiver<MuxEvent>,
    cancel: CancellationToken,
) {
    #[cfg(target_os = "linux")]
    linux_nudge(exec, socket, events, cancel).await;
    #[cfg(not(target_os = "linux"))]
    {
        drop((exec, socket, events));
        cancel.cancelled().await;
    }
}

/// `rig-rs` and private test sockets `rig-rs-<pid>-<n>`.
#[cfg(any(test, target_os = "linux"))]
#[must_use]
pub(crate) fn rig_socket(socket: &str) -> bool {
    socket == "rig-rs" || socket.starts_with("rig-rs-")
}

/// A `list-panes` row that is dead and still has no exit status.
#[cfg(any(test, target_os = "linux"))]
#[must_use]
pub(crate) fn dead_without_status(line: &str) -> bool {
    let mut parts = line.split_whitespace();
    parts.next() == Some("1") && parts.next().is_none()
}

#[cfg(any(test, target_os = "linux"))]
struct Nudge {
    pid: Option<u32>,
    /// Cleared when the server exits so a reused pid is not signaled.
    armed: bool,
}

/// `true` when the caller should reread the server pid.
#[cfg(any(test, target_os = "linux"))]
fn note_event(state: &mut Nudge, event: &MuxEvent) -> bool {
    match event {
        MuxEvent::ServerExited => {
            state.pid = None;
            state.armed = false;
            false
        }
        MuxEvent::Reconnected => {
            state.armed = true;
            state.pid = None;
            true
        }
        MuxEvent::Output { .. }
        | MuxEvent::PaneDied { .. }
        | MuxEvent::WindowClosed { .. }
        | MuxEvent::SessionsChanged
        | MuxEvent::Gap { .. } => false,
    }
}

#[cfg(target_os = "linux")]
async fn linux_nudge(
    exec: Arc<dyn TmuxExec>,
    socket: String,
    mut events: broadcast::Receiver<MuxEvent>,
    cancel: CancellationToken,
) {
    use std::time::Duration;

    if !rig_socket(&socket) {
        tracing::debug!(socket = socket.as_str(), "tmux reap nudge refused");
        return;
    }
    let mut state = Nudge {
        pid: server_pid(exec.as_ref()).await.ok(),
        armed: true,
    };
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            event = events.recv() => {
                if !apply_event(&exec, &mut state, event).await {
                    return;
                }
            }
            () = tokio::time::sleep(Duration::from_millis(15)) => tick(&exec, &mut state).await,
        }
    }
}

#[cfg(target_os = "linux")]
async fn apply_event(
    exec: &Arc<dyn TmuxExec>,
    state: &mut Nudge,
    event: Result<MuxEvent, broadcast::error::RecvError>,
) -> bool {
    match event {
        Ok(event) => {
            if note_event(state, &event) {
                state.pid = server_pid(exec.as_ref()).await.ok();
            }
            true
        }
        Err(broadcast::error::RecvError::Lagged(_)) => {
            refresh_pid(exec, state).await;
            true
        }
        Err(broadcast::error::RecvError::Closed) => false,
    }
}

#[cfg(target_os = "linux")]
async fn refresh_pid(exec: &Arc<dyn TmuxExec>, state: &mut Nudge) {
    if let Ok(pid) = server_pid(exec.as_ref()).await {
        state.pid = Some(pid);
        state.armed = true;
    } else {
        state.pid = None;
        state.armed = false;
    }
}

#[cfg(target_os = "linux")]
async fn tick(exec: &Arc<dyn TmuxExec>, state: &mut Nudge) {
    if !state.armed {
        return;
    }
    let Some(pid) = state.pid else {
        return;
    };
    if let Ok(true) = panes_need_reap(exec.as_ref()).await
        && !signal_chld(pid)
    {
        state.pid = None;
        state.armed = false;
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

#[cfg(target_os = "linux")]
async fn panes_need_reap(exec: &dyn TmuxExec) -> Result<bool, crate::error::MuxError> {
    let out = exec
        .run(&[
            "list-panes".to_owned(),
            "-a".to_owned(),
            "-F".to_owned(),
            "#{pane_dead} #{pane_dead_status}".to_owned(),
        ])
        .await?;
    Ok(out.stdout.lines().any(dead_without_status))
}

/// `true` when `SIGCHLD` was delivered.
#[cfg(target_os = "linux")]
fn signal_chld(pid: u32) -> bool {
    let Ok(raw) = i32::try_from(pid) else {
        return false;
    };
    let Some(pid) = rustix::process::Pid::from_raw(raw) else {
        return false;
    };
    rustix::process::kill_process(pid, rustix::process::Signal::CHILD).is_ok()
}

#[cfg(test)]
mod tests {
    use super::{MuxEvent, Nudge, dead_without_status, note_event, rig_socket};

    #[test]
    fn foreign_socket_is_not_nudged() {
        assert!(rig_socket("rig-rs"));
        assert!(rig_socket("rig-rs-42-7"));
        assert!(!rig_socket("default"));
        assert!(!rig_socket(""));
        assert!(!rig_socket("rig-rsfoo"));
        assert!(!rig_socket("other"));
    }

    #[test]
    fn server_exit_stops_the_nudge_until_reconnect() {
        let mut state = Nudge {
            pid: Some(9),
            armed: true,
        };
        assert!(!note_event(&mut state, &MuxEvent::ServerExited));
        assert!(state.pid.is_none());
        assert!(!state.armed);
        assert!(note_event(&mut state, &MuxEvent::Reconnected));
        assert!(state.armed);
        assert!(state.pid.is_none());
        assert!(!note_event(&mut state, &MuxEvent::SessionsChanged));
        assert!(state.armed);
    }

    #[test]
    fn empty_dead_status_needs_a_nudge() {
        assert!(dead_without_status("1"));
        assert!(dead_without_status("1 "));
        assert!(!dead_without_status("1 0"));
        assert!(!dead_without_status("1 7"));
        assert!(!dead_without_status("0"));
        assert!(!dead_without_status("0 "));
    }
}
