//! Shared daemon state. Cheap to clone into axum tasks.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crewmux_adapters::AdapterRegistry;
use crewmux_mux::Multiplexer;
use tokio_util::sync::CancellationToken;

use crate::error::Error;

/// What one process of the daemon holds.
#[derive(Clone)]
pub(crate) struct App {
    /// SQLite writer.
    pub store: Arc<crewmux_store::Store>,
    /// tmux, or [`crewmux_mux::FakeMux`] in debug builds that set `RIG_FAKE_MUX=1`.
    pub mux: Arc<dyn Multiplexer>,
    /// One mutex per rig name so concurrent `up` calls cannot insert the same seat twice.
    pub up_locks: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    /// Fallback when `RIG_STUB_SCRIPT` is unset. Each launch re-reads the environment.
    pub stub_script: Option<String>,
    /// Shutdown signal.
    pub cancel: CancellationToken,
    /// Milliseconds of the last HTTP request, for the idle reap.
    pub activity: Arc<AtomicU64>,
    /// Registered runtimes. Slice 1 has the stub.
    pub registry: Arc<AdapterRegistry>,
}

impl App {
    /// Remember that a request is in progress so the idle reap waits.
    pub(crate) fn touch(&self) {
        self.activity.store(now_ms(), Ordering::Relaxed);
    }

    /// Serialize `up` for one rig name. The guard is held across the launch awaits.
    pub(crate) async fn lock_rig(&self, name: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let mutex = {
            let mut guard = self.up_locks.lock().await;
            Arc::clone(
                guard
                    .entry(name.to_owned())
                    .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
            )
        };
        mutex.lock_owned().await
    }
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(0))
}

pub(crate) fn socket_name_from_env() -> String {
    std::env::var("RIG_TMUX_SOCKET").unwrap_or_else(|_| "rig-rs".to_owned())
}

pub(crate) fn stub_script_from_env() -> Option<String> {
    std::env::var("RIG_STUB_SCRIPT")
        .ok()
        .filter(|value| !value.is_empty())
}

pub(crate) fn stub_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("RIG_STUB_BIN") {
        return PathBuf::from(path);
    }
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|dir| dir.join("rig-stub-agent")))
        .unwrap_or_else(|| PathBuf::from("rig-stub-agent"))
}

pub(crate) fn idle_exceeded(activity: &AtomicU64) -> bool {
    let then = activity.load(Ordering::Relaxed);
    now_ms().saturating_sub(then) > 180_000
}

/// Map a join error from `spawn_blocking`.
#[expect(
    clippy::needless_pass_by_value,
    reason = "map_err passes JoinError by value and Display does not consume it"
)]
pub(crate) fn join_err(err: tokio::task::JoinError) -> Error {
    Error::Message(err.to_string())
}
