//! Daemon API contract: request and response types, route table and typed client.
//!
//! Slice 0b added the blocking HTTP/1.1 client. Slice 13 adds the read models
//! (`ListSeats`, `AttachInfo`, hydrate snapshot) and the `GET /api/events` path.
//! Slice 1 must serve the read routes and reconcile `StreamEvent` with its events.

mod client;
mod endpoint;
mod error;
mod model;
mod read;

pub use client::{Client, Response, daemon_socket};
pub use crewmux_types::{
    LauncherKind, PodName, ReadinessCode, RigId, RigName, RuntimeKind, SeatId, SeatName, SeatState,
    SessionId,
};
pub use endpoint::{
    AttachInfoRoute, Endpoint, Events, Hydrate, ListSeats, MAX_SUN_PATH_BYTES, Method, attach_path,
    daemon_bind_path, fit_sun_path, path_bytes, tui_socket_path,
};
pub use error::Error;
pub use model::{
    ActivityItem, AttachInfo, HealthItem, HealthSummary, LauncherConfig, SeatList, SeatView,
    Snapshot, StreamEvent,
};
