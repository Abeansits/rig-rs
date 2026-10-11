//! Daemon API contract: request and response types, route table and typed client.
//!
//! Slice 0b adds the blocking HTTP/1.1 client. The rig-rs daemon is reached on
//! the Unix socket at `$RIG_HOME/run/daemon.sock`. TCP exists so a harness can
//! reject it for this side. No new daemon routes are declared here.

mod client;
mod endpoint;
mod error;

pub use client::{Client, Response, daemon_socket, rig_home};
pub use endpoint::{
    AttachInfo, AttachRequest, CaptureRequest, CaptureSeat, CreateRigUp, DownRequest, Endpoint,
    Healthz, ListSeats, RigDown,
};
pub use error::Error;
