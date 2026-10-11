//! Terminal multiplexer abstraction and its tmux implementation.
//!
//! [`TmuxMux`] speaks argv-only tmux on a private `-L` socket. [`FakeMux`] records
//! the same argv for tests. Real tmux integration tests are behind the `tmux-it`
//! feature.

mod argv;
mod clients;
mod control;
mod control_line;
mod death;
mod decode;
mod error;
mod exec;
mod fake;
mod fake_script;
mod hashutil;
mod hub;
mod parse;
mod paste;
mod reap;
mod session_kill;
mod tmux_mux;
mod types;

pub use crewmux_types::Key;
pub use crewmux_types::tmux_version::{self, Version, parse_version};
pub use error::MuxError;
pub use fake::FakeMux;
pub use tmux_mux::{TmuxConfig, TmuxMux, socket_owned_by};
pub use types::{
    CONTROL_SESSION, Capture, CaptureRange, ClientInfo, DeathStream, Geometry, GuardInputs,
    MuxEvent, OptScope, OutputStream, PANE_RING_BYTES, PaneInfo, PaneRef, PasteOpts, PasteReceipt,
    SessionSpec,
};

use tokio::sync::broadcast;

/// Async tmux (or fake) operations used by the daemon.
#[async_trait::async_trait]
pub trait Multiplexer: Send + Sync + 'static {
    /// Create a detached session and return its pane, including `pane_id`.
    async fn create_session(&self, spec: &SessionSpec) -> Result<PaneRef, MuxError>;

    /// Exact-match session existence. Unknown sessions are `Ok(false)`.
    async fn has_session(&self, name: &str) -> Result<bool, MuxError>;

    /// Kill a session. Missing sessions are [`MuxError::NotFound`].
    async fn kill_session(&self, name: &str) -> Result<(), MuxError>;

    /// Every user pane. The hidden control session is omitted.
    async fn list_panes(&self) -> Result<Vec<PaneInfo>, MuxError>;

    /// Deliver keys. A literal containing a newline is rejected; use [`paste`](Self::paste).
    async fn send_keys(&self, pane: &PaneRef, keys: &[Key]) -> Result<(), MuxError>;

    /// Paste text with `load-buffer` / `paste-buffer`. Does not send Enter.
    async fn paste(
        &self,
        pane: &PaneRef,
        text: &str,
        opts: PasteOpts,
    ) -> Result<PasteReceipt, MuxError>;

    /// Capture the pane and a stable hash of the text and cursor.
    async fn capture(&self, pane: &PaneRef, range: CaptureRange) -> Result<Capture, MuxError>;

    /// Live `%output` for one pane. The pane id must already be known.
    async fn subscribe_output(&self, pane: &PaneRef) -> Result<OutputStream, MuxError>;

    /// `set-option` at the given scope.
    async fn set_option(&self, scope: OptScope, key: &str, value: &str) -> Result<(), MuxError>;

    /// Clients with `control_mode == false` on `session`.
    async fn human_clients(&self, session: &str) -> Result<Vec<ClientInfo>, MuxError>;

    /// `#{pane_in_mode}`.
    async fn pane_in_mode(&self, pane: &PaneRef) -> Result<bool, MuxError>;

    /// tmux-scoped guard inputs. The verdict is computed in a later slice.
    async fn guard_inputs(&self, pane: &PaneRef, tail_lines: u16) -> Result<GuardInputs, MuxError>;

    /// Pane-death and window-close events from here on.
    fn death_events(&self) -> DeathStream;

    /// Resolved tmux binary and the server socket path (`-S`), for `rig attach`.
    fn attach_program(&self) -> (String, String);

    /// Every multiplexer event from here on, including output.
    fn events(&self) -> broadcast::Receiver<MuxEvent>;
}
