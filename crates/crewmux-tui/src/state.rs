// Derived from openrig packages/tui/src/types.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
//! Elm state. `update` is the only place this changes.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use crewmux_api::{LauncherKind, PodName, RigName, SeatKey, Snapshot};

use crate::action::HitMap;

/// How long a row stays flashed after a live event. Matches the TypeScript window.
pub const FLASH_WINDOW: Duration = Duration::from_millis(600);

/// Clock tick when nothing is flashing. Long enough that idle CPU stays under 1%.
pub const IDLE_TICK: Duration = Duration::from_secs(1);

/// Left-hand navigator, seat detail, attention, health, or recent activity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    /// Rigs, pods, and seats.
    Explorer,
    /// The selected seat.
    Detail,
    /// Seats in `Attention`.
    Attention,
    /// Health findings, or an empty state when the daemon has none.
    Health,
    /// Recent activity lines.
    Activity,
}

impl Section {
    /// Parse a control-socket section name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "explorer" | "topology" => Some(Self::Explorer),
            "detail" | "seat" => Some(Self::Detail),
            "attention" => Some(Self::Attention),
            "health" => Some(Self::Health),
            "activity" | "recent" => Some(Self::Activity),
            _ => None,
        }
    }

    /// Wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Explorer => "explorer",
            Self::Detail => "detail",
            Self::Attention => "attention",
            Self::Health => "health",
            Self::Activity => "activity",
        }
    }
}

/// Identity of an explorer row.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RowKey {
    /// A rig.
    Rig(RigName),
    /// A pod inside a rig.
    Pod {
        /// Owning rig.
        rig: RigName,
        /// Pod name.
        pod: PodName,
    },
    /// A seat.
    Seat(SeatKey),
}

/// Where this TUI is allowed to attach. Read at startup and stored so `update` stays pure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionEnv {
    /// tmux socket name passed to `-L`. The product default is `rig-rs`.
    pub socket_name: String,
    /// Basename of `$TMUX`'s socket when the TUI is already inside tmux.
    pub inside_tmux: Option<String>,
    /// Configured external launcher, if the operator set one.
    pub launcher: Option<LauncherKind>,
}

impl Default for SessionEnv {
    fn default() -> Self {
        Self {
            socket_name: "rig-rs".to_owned(),
            inside_tmux: None,
            launcher: None,
        }
    }
}

/// One-shot highlight after a live event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowFlash {
    /// Row to invert.
    pub key: RowKey,
    /// When the flash started.
    pub at: Instant,
}

/// Connection the header is allowed to claim. `Live` is not a variant: the
/// header says "live" only while [`ConnState::Connected`] and not stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnState {
    /// The event stream is up.
    Connected,
    /// The stream dropped and has not started another attempt.
    Dropped,
    /// A reconnect attempt is in progress.
    Reconnecting,
    /// The stream is not offered, or it answered with a non-event body.
    Unavailable,
}

/// Honest load lifecycle. Data absence is not a connection fact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadState {
    /// When the last hydrate succeeded.
    pub last_success_at: Option<Instant>,
    /// The snapshot may be older than the daemon.
    pub stale: bool,
    /// Event-stream connection.
    pub connection: ConnState,
    /// A hydrate is running.
    pub in_flight: bool,
    /// At least one hydrate has finished, successfully or not.
    pub settled: bool,
}

impl LoadState {
    /// Before the first hydrate.
    #[must_use]
    pub const fn initial() -> Self {
        Self {
            last_success_at: None,
            stale: false,
            connection: ConnState::Unavailable,
            in_flight: false,
            settled: false,
        }
    }

    /// Connection word painted in the header.
    #[must_use]
    pub const fn connection_label(&self) -> &'static str {
        match self.connection {
            ConnState::Connected => "connected",
            ConnState::Dropped => "dropped",
            ConnState::Reconnecting => "reconnecting",
            ConnState::Unavailable => "unavailable",
        }
    }

    /// The header may say "live" only while the stream is up and the snapshot is fresh.
    #[must_use]
    pub const fn shows_live(&self) -> bool {
        matches!(self.connection, ConnState::Connected) && !self.stale
    }
}

/// Terminal size in cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    /// Columns.
    pub cols: u16,
    /// Rows.
    pub rows: u16,
}

impl Size {
    /// The acceptance goldens use 120×40 and 80×24.
    #[must_use]
    pub const fn new(cols: u16, rows: u16) -> Self {
        Self { cols, rows }
    }
}

/// Whole TUI state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    /// Which content pane is showing.
    pub section: Section,
    /// Selected explorer row, if any.
    pub selection: Option<RowKey>,
    /// Last good snapshot.
    pub snapshot: Snapshot,
    /// Load lifecycle.
    pub load: LoadState,
    /// Content-pane scroll, in lines.
    pub scroll: u16,
    /// Rows flashing right now.
    pub flashes: Vec<RowFlash>,
    /// Last known terminal size.
    pub size: Size,
    /// Logical clock. `Tick` advances it; `update` does not read the wall clock.
    pub now: Instant,
    /// Disclosure keys that are open.
    pub expanded: BTreeSet<RowKey>,
    /// One-line notice, such as a refused attach.
    pub notice: Option<String>,
    /// Scroll limit from the last layout. Wheel input clamps to this.
    pub content_max_offset: u16,
    /// Hit targets from the last frame. Mouse input uses this, not a parallel layout.
    pub hits: HitMap,
    /// Explorer pane width from the last frame, in 1-based columns (the right edge).
    pub explorer_right: u16,
    /// Disclosure keys the operator collapsed. Later hydrates do not force them open.
    pub collapsed: BTreeSet<RowKey>,
    /// Attach policy for this process.
    pub env: SessionEnv,
    /// Highest live-event id applied. Events at or below this id are dropped.
    pub last_event_id: Option<u64>,
}

impl State {
    /// Empty instance at `size`, clock frozen at `now`.
    #[must_use]
    pub fn new(size: Size, now: Instant) -> Self {
        Self {
            section: Section::Explorer,
            selection: None,
            snapshot: Snapshot::empty(),
            load: LoadState::initial(),
            scroll: 0,
            flashes: Vec::new(),
            size,
            now,
            expanded: BTreeSet::new(),
            notice: None,
            content_max_offset: 0,
            hits: HitMap::default(),
            explorer_right: explorer_right(size.cols),
            collapsed: BTreeSet::new(),
            env: SessionEnv::default(),
            last_event_id: None,
        }
    }
}

/// Right edge of the explorer pane, 1-based, matching `explorerWidth` from the TypeScript layout.
#[must_use]
pub fn explorer_right(cols: u16) -> u16 {
    let quarter = (u32::from(cols).saturating_add(2)) / 4;
    let clamped = quarter.clamp(24, 32);
    u16::try_from(clamped).unwrap_or(32)
}

/// How long the loop should sleep. Idle sleeps a full second; a flash wakes at its deadline.
#[must_use]
pub fn idle_wait(state: &State) -> Duration {
    let mut wait = IDLE_TICK;
    for flash in &state.flashes {
        let deadline = flash.at.checked_add(FLASH_WINDOW).unwrap_or(flash.at);
        let remaining = deadline.saturating_duration_since(state.now);
        if remaining < wait {
            wait = remaining;
        }
    }
    if wait.is_zero() {
        Duration::from_millis(1)
    } else {
        wait
    }
}
