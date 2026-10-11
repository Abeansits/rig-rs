//! Daemon API contract: request and response types, route table and typed client.
//!
//! Slice 0b adds the blocking HTTP/1.1 client. The rig-rs daemon is reached on
//! the Unix socket at `$RIG_HOME/run/daemon.sock`. TCP exists so a harness can
//! reject it for this side. No new daemon routes are declared here.

mod client;
mod dirs;
mod endpoint;
mod error;
mod stream;

pub use client::{Client, Response, daemon_socket, rig_home};
pub use dirs::{create_private, open_private};
pub use endpoint::{
    AskSeat, AttachInfo, AttachRequest, BroadcastSend, CaptureRequest, CaptureSeat, CreateRigUp,
    DownRequest, Endpoint, Healthz, ListSeats, QueueClaim, QueueCollection, QueueFallback,
    QueueHandoff, QueueShow, RigDown, SendMessage, UnclaimItem, Whoami,
};
pub use error::Error;
pub use stream::{EventStream, StreamEvent, StreamKind, StreamPayload};
