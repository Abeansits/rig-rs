//! Daemon API contract: request and response types, route table and typed client.
//!
//! Slice 0b adds the blocking HTTP/1.1 client. The rig-rs daemon is reached on
//! the Unix socket at `$RIG_HOME/run/daemon.sock`. TCP exists so a harness can
//! reject it for this side. No new daemon routes are declared here.

mod chat;
mod client;
mod dirs;
mod endpoint;
mod error;

pub use chat::{
    CHAT_MESSAGE_TYPE, ChatClear, ChatHistory, ChatSend, ChatTopic, ChatWatch, ChatWire,
    RigSummary, RigSummaryRow, StreamEvent,
};
pub use client::{Client, HttpStream, Response, daemon_socket, rig_home};
pub use dirs::{create_private, open_private};
pub use endpoint::{
    AttachInfo, AttachRequest, CaptureRequest, CaptureSeat, CreateRigUp, DownRequest, Endpoint,
    Healthz, ListSeats, RigDown,
};
pub use error::Error;
