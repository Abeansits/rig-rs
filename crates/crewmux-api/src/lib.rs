//! Daemon API contract: request and response types, route table and typed client.
//!
//! Slice 1 owns the route table (`GET /api/seats`, `POST /api/seats/attach`).
//! The TUI builds its snapshot from that list and asks the daemon for attach
//! argv. `StreamEvent` is the slice 4 envelope the live tail decodes.

mod chat;
mod client;
mod dirs;
mod endpoint;
mod error;
mod model;
mod read;
mod stream;

pub use chat::{
    CHAT_MESSAGE_TYPE, ChatClear, ChatHistory, ChatSend, ChatTopic, ChatWatch, ChatWatchEvent,
    ChatWire, MAX_CHAT_BODY, RigSummary, RigSummaryRow,
};
pub use client::{
    Client, HttpStream, MAX_SUN_PATH_BYTES, Response, daemon_bind_path, daemon_socket, path_bytes,
    rig_home, tui_socket, tui_socket_path,
};
pub use crewmux_core::SeatState;
pub use crewmux_types::{
    LauncherKind, MAX_TOPIC_LEN, PodName, ReadinessCode, RigName, RuntimeKind, SeatKey, SeatName,
    TmuxSession,
};
pub use dirs::{create_private, open_private};
pub use endpoint::{
    AskSeat, AttachInfo, AttachRequest, BroadcastSend, CaptureRequest, CaptureSeat, CreateRigUp,
    DownRequest, Endpoint, Healthz, ListSeats, QueueClaim, QueueCollection, QueueFallback,
    QueueHandoff, QueueShow, RigDown, SendMessage, UnclaimItem, Whoami,
};
pub use error::Error;
pub use model::{ActivityItem, HealthItem, HealthSummary, LauncherConfig, SeatView, Snapshot};
pub use stream::{EventStream, StreamEvent, StreamKind, StreamPayload};
