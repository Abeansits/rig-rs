//! Shared daemon state. Cheap to clone into axum tasks.

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
    /// tmux, or [`crewmux_mux::FakeMux`] when `RIG_FAKE_MUX=1`.
    pub mux: Arc<dyn Multiplexer>,
    /// `-L` socket name passed to `tmux attach`.
    pub socket_name: String,
    /// `RIG_STUB_SCRIPT`, captured at startup so tests can inject it without `set_var`.
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
