//! Temporary live-event fan-out.
//!
//! Slice 4 owns `GET /api/events` and the transcript writer. This module is the
//! stand-in until that spec lands: SSE on the ADR-0006 Unix socket, no replay,
//! no transcript file. The TUI never links this crate. It only decodes
//! [`crewmux_api::StreamEvent`].

mod error;
mod hub;
mod serve;

pub use error::Error;
pub use hub::EventHub;
pub use serve::serve;

#[cfg(test)]
mod live;
