//! Temporary live-event fan-out.
//!
//! Slice 4 owns `GET /api/events` and the transcript writer. This module is the
//! stand-in: SSE of the slice 4 envelope on the same path, no backlog, no
//! transcript file. A client that sends `Last-Event-ID` or `last_event_id`
//! receives one `gap` event, then live events. The TUI never links this crate.

mod error;
mod hub;
mod serve;

pub use error::Error;
pub use hub::EventHub;
pub use serve::serve;

#[cfg(test)]
mod live;
