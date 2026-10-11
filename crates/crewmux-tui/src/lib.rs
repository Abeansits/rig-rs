//! Terminal UI backed by the daemon API client.
//!
//! The live stream is consumed as [`crewmux_api::StreamEvent`] over `GET /api/events`.
//! This crate does not depend on the daemon. Slice 4 owns that route and can replace
//! the temporary fan-out without a TUI change, as long as the client event names stay.

mod action;
mod app;
mod attach;
mod control;
mod error;
mod guard;
mod input;
mod msg;
mod refresh;
mod rows;
mod seat_bits;
mod sse;
mod state;
mod update;
mod view;

pub use action::{Action, CrossLink, HitMap, HitTarget};
pub use app::{Config, run, run_blocking};
pub use attach::{LaunchFail, LaunchFn, LaunchRequest, perform, tmux_argv};
pub use error::Error;
pub use guard::{CrosstermGuard, TerminalGuard};
pub use input::{
    DecodedMouse, Dispatch, HitAction, MouseAction, Perform, decode_mice, dispatch_sgr,
    resolve_mouse_action, sgr_click,
};
pub use msg::{
    AttachOutcome, AttachTarget, Cmd, ControlCmd, ControlReplyId, KeyEvent, KeyKind, MouseEvent,
    Msg,
};
pub use state::{
    ConnState, FLASH_WINDOW, IDLE_TICK, LoadState, RowFlash, RowKey, Section, SessionEnv, Size,
    State, explorer_right, idle_wait,
};
pub use update::update;
pub use view::view;

#[cfg(test)]
mod fake_api;
#[cfg(test)]
mod hitmap_tests;
#[cfg(test)]
mod parity_tests;
#[cfg(test)]
mod render_tests;
#[cfg(test)]
mod socket_tests;
