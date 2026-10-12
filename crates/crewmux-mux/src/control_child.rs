//! Pid of the live `tmux -C` control client.
//!
//! tmux forks the server and the server is reparented to init, with the same
//! argv as `new-session -A`. Killing the daemon does not kill that server.
//! Shutdown records the client pid, kills it, and waits so a reconnect cannot
//! start another server after the cancellation token has fired.

use std::io;
use std::path::PathBuf;

use tokio::process::Child;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::exec::control_command;

/// The control client this process spawned, if it is still running.
pub(crate) struct ControlSlot {
    pid: Mutex<Option<u32>>,
}

/// Why [`ControlSlot::spawn`] did not start a client.
pub(crate) enum SpawnError {
    /// Shutdown already cancelled the token. No process was spawned.
    Cancelled,
    /// The OS rejected the spawn.
    Io(io::Error),
}

impl ControlSlot {
    pub(crate) fn new() -> Self {
        Self {
            pid: Mutex::new(None),
        }
    }

    /// Spawn the client unless `cancel` has fired.
    ///
    /// The pid is published under the same lock as the cancel check, so
    /// [`kill_and_wait`](Self::kill_and_wait) cannot miss a child that spawn
    /// already created. The lock is not held across `.await`.
    pub(crate) async fn spawn(
        &self,
        binary: &PathBuf,
        socket: &str,
        cancel: &CancellationToken,
    ) -> Result<Child, SpawnError> {
        let mut guard = self.pid.lock().await;
        if cancel.is_cancelled() {
            return Err(SpawnError::Cancelled);
        }
        let child = control_command(binary, socket)
            .spawn()
            .map_err(SpawnError::Io)?;
        if let Some(pid) = child.id() {
            *guard = Some(pid);
        }
        Ok(child)
    }

    /// Pid of the client spawned by [`spawn`](Self::spawn), if it is still recorded.
    pub(crate) async fn id(&self) -> Option<u32> {
        *self.pid.lock().await
    }

    /// Forget `pid` after this task has reaped it.
    pub(crate) async fn clear(&self, pid: u32) {
        let mut guard = self.pid.lock().await;
        if *guard == Some(pid) {
            *guard = None;
        }
    }

    #[cfg(test)]
    fn record_for_test(&self, pid: u32) {
        *self.pid.blocking_lock() = Some(pid);
    }

    /// SIGKILL the client and wait until it is gone.
    ///
    /// Call from a std thread. `blocking_lock` panics inside a tokio worker.
    pub(crate) fn kill_and_wait(&self) {
        let pid = self.pid.blocking_lock().take();
        let Some(pid) = pid else {
            return;
        };
        reap(pid);
    }
}

fn reap(pid: u32) {
    let Ok(raw) = i32::try_from(pid) else {
        return;
    };
    let Some(pid) = rustix::process::Pid::from_raw(raw) else {
        return;
    };
    let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
    // SIGKILL cannot be caught. A blocking wait reaps the child; `ECHILD`
    // means another waiter already did. This runs on a std thread, never a
    // tokio worker.
    loop {
        match rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::empty()) {
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Ok(_) | Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ControlSlot;

    #[test]
    fn kill_and_wait_reaps_the_child() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("sleep");
        let slot = ControlSlot::new();
        slot.record_for_test(child.id());
        slot.kill_and_wait();
        if let Ok(None) = child.try_wait() {
            drop(child.kill());
            drop(child.wait());
            panic!("child still running");
        }
    }
}
