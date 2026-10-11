//! Multiplexer DTOs shared by the tmux driver and [`crate::FakeMux`].

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use bytes::Bytes;

use crate::hashutil::screen_hash;

/// Hidden session the control-mode client attaches to.
///
/// User seats are separate sessions so this client cannot resize them.
pub const CONTROL_SESSION: &str = "crewmux-control";

/// Per-pane output retained for a slow subscriber, in bytes.
pub const PANE_RING_BYTES: usize = 64 * 1024;

/// Visible size of a pane, in cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    /// Columns.
    pub cols: u16,
    /// Rows.
    pub rows: u16,
}

impl Default for Geometry {
    fn default() -> Self {
        Self { cols: 90, rows: 27 }
    }
}

/// A pane address. `pane_id` (`%N`) is set once tmux has assigned one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneRef {
    /// Session name, as tmux stored it.
    pub session: String,
    /// Window index, when known.
    pub window: Option<u32>,
    /// Pane index within the window, when known.
    pub pane: Option<u32>,
    /// Stable pane id (`%N`), when known.
    pub pane_id: Option<String>,
}

impl PaneRef {
    /// Address a session's current pane before its id is known.
    #[must_use]
    pub fn session(name: impl Into<String>) -> Self {
        Self {
            session: name.into(),
            window: None,
            pane: None,
            pane_id: None,
        }
    }
}

/// How to create one detached tmux session.
#[derive(Clone, Debug)]
pub struct SessionSpec {
    /// Requested session name. `.` and `:` are stored as `_`, matching tmux.
    pub name: String,
    /// Working directory for the pane.
    pub cwd: PathBuf,
    /// Environment entries passed with `-e` (sorted by key).
    pub env: BTreeMap<String, String>,
    /// Command argv. Empty starts the default shell. One element may be a shell line tmux runs.
    pub argv: Vec<String>,
    /// Initial window size. Defaults to 90×27.
    pub geometry: Geometry,
    /// Keep the pane after the process exits so a crash remains visible.
    pub remain_on_exit: bool,
}

impl SessionSpec {
    /// A session with the default geometry and no extra environment.
    #[must_use]
    pub fn new(name: impl Into<String>, cwd: PathBuf) -> Self {
        Self {
            name: name.into(),
            cwd,
            env: BTreeMap::new(),
            argv: Vec::new(),
            geometry: Geometry::default(),
            remain_on_exit: false,
        }
    }
}

/// Flags for [`crate::Multiplexer::paste`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PasteOpts {
    /// Pass `-p` so tmux wraps the paste in bracketed-paste markers.
    pub bracketed: bool,
    /// Pass `-r` so newlines stay LF instead of becoming CR.
    pub preserve_lf: bool,
}

impl Default for PasteOpts {
    fn default() -> Self {
        Self {
            bracketed: true,
            preserve_lf: true,
        }
    }
}

/// Result of a successful paste. Enter is not part of the paste.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasteReceipt {
    /// tmux buffer name (`rig-<ulid>`).
    pub buffer: String,
    /// Payload length in bytes.
    pub bytes: usize,
    /// Whether `-p` was requested.
    pub bracketed: bool,
}

/// Which part of the pane history to capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureRange {
    /// The visible screen.
    Visible,
    /// The last `n` lines, via `capture-pane -S -n`.
    LastLines(u16),
    /// The last `n` lines of history, via `capture-pane -S -n`.
    History(u32),
}

/// A screen snapshot plus a stable hash of text and cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    /// Captured text, as tmux printed it.
    pub text: String,
    /// Zero-based cursor `(column, row)`.
    pub cursor: (u16, u16),
    /// FNV-1a of `text` and `cursor`. Stable for an unchanged screen.
    pub hash: u64,
}

impl Capture {
    /// Build a capture and its hash.
    #[must_use]
    pub fn new(text: String, cursor: (u16, u16)) -> Self {
        let hash = screen_hash(&text, cursor);
        Self { text, cursor, hash }
    }
}

/// One pane row from `list-panes`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneInfo {
    /// Pane id (`%N`).
    pub pane_id: String,
    /// Window id (`@N`).
    pub window_id: String,
    /// Session name.
    pub session: String,
    /// Process id of the pane command.
    pub pid: u32,
    /// Foreground command name.
    pub current_cmd: String,
    /// `pane_dead`.
    pub dead: bool,
    /// `pane_dead_status`, when tmux reported one.
    pub dead_status: Option<i32>,
    /// Pane size in cells.
    pub size: Geometry,
    /// `pane_in_mode` (copy mode or another mode).
    pub in_mode: bool,
}

/// One tmux client from `list-clients`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientInfo {
    /// Client tty. Empty for the control-mode client on some versions.
    pub tty: String,
    /// Session the client is attached to.
    pub session: String,
    /// `client_activity` at 1-second resolution.
    pub activity: SystemTime,
    /// `client_control_mode`. Our control client is `true` and is not a human.
    pub control_mode: bool,
}

/// Push event from the control-mode client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MuxEvent {
    /// Bytes reconstructed from `%output` for one pane.
    Output {
        /// Pane id.
        pane_id: String,
        /// Decoded bytes.
        bytes: Bytes,
    },
    /// The pane's process exited, or the window that held it closed.
    PaneDied {
        /// Pane id.
        pane_id: String,
        /// Exit status, when tmux reported `pane_dead_status`.
        status: Option<i32>,
    },
    /// A window closed (`%window-close` or `%unlinked-window-close`).
    WindowClosed {
        /// Window id (`@N`).
        window_id: String,
        /// Session that owned the window, when known.
        session: String,
    },
    /// `%sessions-changed`.
    SessionsChanged,
    /// The tmux server exited (`%exit` or EOF after `kill-server`).
    ServerExited,
    /// The control client attached again and rebuilt pane state.
    Reconnected,
    /// A subscriber fell behind the 64 KiB per-pane buffer.
    Gap {
        /// Pane whose bytes were dropped.
        pane_id: String,
        /// How many bytes this subscriber missed.
        dropped: usize,
    },
}

/// tmux-scope inputs of the human-input guard. The verdict is slice 4.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuardInputs {
    /// Human clients only (`control_mode == false`).
    pub human_clients: Vec<ClientInfo>,
    /// Newest `client_activity` among human clients, if any.
    pub last_human_keystroke_tmux: Option<SystemTime>,
    /// Filled by the daemon from terminal input taps (slice 4). Always `None` here.
    pub last_human_keystroke_tap: Option<Instant>,
    /// `#{pane_in_mode}`.
    pub pane_in_mode: bool,
    /// `capture(LastLines(n))` for the composer parser.
    pub tail: Capture,
    /// When this multiplexer last pasted or sent keys.
    pub last_own_write: Option<Instant>,
}

/// Where `set-option` applies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptScope {
    /// Global option (`-g`). `window-size` is rejected here.
    Global,
    /// Server option (`-s`).
    Server,
    /// Session option. The name is canonicalised.
    Session(String),
    /// Window option (`-w`) for an exact target.
    Window(String),
    /// Pane option (`-p`).
    Pane(PaneRef),
}

/// Live `%output` for one pane.
#[derive(Debug)]
pub struct OutputStream {
    pub(crate) pane_id: String,
    pub(crate) rx: tokio::sync::mpsc::Receiver<MuxEvent>,
    pub(crate) dropped: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl OutputStream {
    /// Next output or gap event. `None` when the multiplexer has shut down.
    pub async fn recv(&mut self) -> Option<MuxEvent> {
        if let Ok(event) = self.rx.try_recv() {
            return Some(event);
        }
        if let Some(gap) = self.take_gap() {
            return Some(gap);
        }
        if let Some(event) = self.rx.recv().await {
            return Some(event);
        }
        self.take_gap()
    }

    fn take_gap(&mut self) -> Option<MuxEvent> {
        let missed = self.dropped.swap(0, std::sync::atomic::Ordering::AcqRel);
        if missed > 0 {
            Some(MuxEvent::Gap {
                pane_id: self.pane_id.clone(),
                dropped: missed,
            })
        } else {
            None
        }
    }
}

/// Pane-death and window-close events filtered from [`MuxEvent`].
#[derive(Debug)]
pub struct DeathStream {
    pub(crate) rx: tokio::sync::broadcast::Receiver<MuxEvent>,
}

impl DeathStream {
    /// Next death or window-close event. `None` when the bus is closed.
    pub async fn recv(&mut self) -> Option<MuxEvent> {
        loop {
            match self.rx.recv().await {
                Ok(event @ (MuxEvent::PaneDied { .. } | MuxEvent::WindowClosed { .. })) => {
                    return Some(event);
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}
