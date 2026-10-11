//! Messages and commands for the Elm loop.

use crewmux_api::{SeatKey, Snapshot, TmuxSession};
use serde_json::Value;

use crate::action::HitMap;
use crate::state::{ConnState, Section};

/// Why a hydrate failed. The snapshot stays as it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HydrateFail {
    /// The daemon socket did not answer.
    Unreachable,
    /// The body was not a snapshot.
    BadBody,
}

/// Keyboard input after the terminal decoder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    /// Which key.
    pub kind: KeyKind,
}

/// Keys v1 handles. Anything else is ignored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyKind {
    /// A printable character.
    Char(char),
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Enter.
    Enter,
}

/// One decoded mouse event. Coordinates are 1-based, matching the TypeScript hit map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseEvent {
    /// Button code. `0` is the primary button. `64` and `65` are wheel notches.
    pub button: u16,
    /// 1-based column.
    pub x: u16,
    /// 1-based row.
    pub y: u16,
}

/// Correlation id for one control-socket command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlReplyId(pub u64);

/// v1 control-socket verbs. Unknown verbs are still delivered so `update` replies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlCmd {
    /// Select a seat row.
    Select {
        /// Seat token.
        seat: SeatKey,
        /// Reply id.
        reply: ControlReplyId,
    },
    /// Switch section.
    Section {
        /// Parsed section.
        section: Section,
        /// Reply id.
        reply: ControlReplyId,
    },
    /// Ask for a hydrate.
    Refresh {
        /// Reply id.
        reply: ControlReplyId,
    },
    /// Attach a seat.
    Attach {
        /// Seat token.
        seat: SeatKey,
        /// Reply id.
        reply: ControlReplyId,
    },
    /// A verb this build does not implement.
    Unknown {
        /// The verb, for the error text.
        verb: String,
        /// Reply id.
        reply: ControlReplyId,
    },
}

/// What to attach, once `update` has applied the tmux-nesting rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttachTarget {
    /// `tmux -L <socket> attach -t =<session>`.
    Tmux {
        /// Socket name for `-L`.
        socket: String,
        /// Session name.
        session: TmuxSession,
        /// Seat name sent to `POST /api/seats/attach`.
        seat: SeatKey,
    },
    /// `tmux switch-client -t =<session>` on the server we are already inside.
    SwitchClient {
        /// Socket name for `-L`.
        socket: String,
        /// Session name.
        session: TmuxSession,
    },
    /// Herdr or Ghostty. The integrations crate performs the launch.
    /// `socket` and `session` are only used if that launch fails and the TUI falls back to tmux.
    Launcher {
        /// Which launcher.
        kind: crewmux_api::LauncherKind,
        /// Seat being opened.
        seat: SeatKey,
        /// tmux socket name for the fallback attach.
        socket: String,
        /// tmux session for the fallback attach.
        session: TmuxSession,
    },
}

/// Result of one external attach.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttachOutcome {
    /// The client returned and the session is still there.
    Detached,
    /// A launcher failed and the TUI fell back to tmux attach.
    DetachedWithNotice(String),
    /// The command failed before a client was attached.
    Failed(String),
    /// The session disappeared while we were attached.
    SessionGone,
}

/// One input to `update`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Msg {
    /// Key press.
    Key(KeyEvent),
    /// Mouse press or wheel notch.
    Mouse(MouseEvent),
    /// Terminal size changed.
    Resize(u16, u16),
    /// A hydrate finished.
    Hydrated(Result<Snapshot, HydrateFail>),
    /// One live event, applied in place.
    Live(crewmux_api::StreamEvent),
    /// Event-stream connection changed.
    Conn(ConnState),
    /// Clock tick. Advances the logical clock by [`crate::state::IDLE_TICK`].
    Tick,
    /// External attach finished.
    AttachReturned(AttachOutcome),
    /// Control-socket command.
    Control(ControlCmd),
    /// Hit map from the frame that was just painted.
    Painted(HitMap),
}

/// Side effect requested by `update`. The app performs these; `update` does not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    /// Read a fresh snapshot.
    Hydrate,
    /// Suspend, attach, resume.
    Attach(AttachTarget),
    /// Leave the TUI.
    Quit,
    /// Write one control-socket reply.
    Reply(ControlReplyId, Value),
}
