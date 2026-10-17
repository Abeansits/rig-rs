//! Daemon routes the CLI calls. Each one is an [`Endpoint`].

use serde::{Deserialize, Serialize};

/// A daemon route the CLI calls.
pub trait Endpoint {
    /// HTTP method.
    const METHOD: &'static str;
    /// Absolute path.
    const PATH: &'static str;
}

/// `GET /healthz`.
#[derive(Debug)]
pub struct Healthz;

impl Endpoint for Healthz {
    const METHOD: &'static str = "GET";
    const PATH: &'static str = "/healthz";
}

/// `POST /api/rigs/up`. The body is the spec YAML, not JSON.
#[derive(Debug)]
pub struct CreateRigUp;

impl Endpoint for CreateRigUp {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/rigs/up";
}

/// `GET /api/seats`.
#[derive(Debug)]
pub struct ListSeats;

impl Endpoint for ListSeats {
    const METHOD: &'static str = "GET";
    const PATH: &'static str = "/api/seats";
}

/// `POST /api/seats/capture`.
#[derive(Debug)]
pub struct CaptureSeat;

impl Endpoint for CaptureSeat {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/seats/capture";
}

/// `POST /api/seats/attach`.
#[derive(Debug)]
pub struct AttachInfo;

impl Endpoint for AttachInfo {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/seats/attach";
}

/// `POST /api/seats/submit`.
#[derive(Debug)]
pub struct SubmitSeat;

impl Endpoint for SubmitSeat {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/seats/submit";
}

/// Body of [`SubmitSeat`].
#[derive(Debug, Deserialize, Serialize)]
pub struct SubmitRequest {
    /// Seat name.
    pub seat: String,
    /// Turn text. The daemon pastes it and sends Enter.
    pub text: String,
}

/// `POST /api/rigs/down`.
#[derive(Debug)]
pub struct RigDown;

impl Endpoint for RigDown {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/rigs/down";
}

/// Body of [`CaptureSeat`].
#[derive(Debug, Deserialize, Serialize)]
pub struct CaptureRequest {
    /// Seat name.
    pub seat: String,
    /// When set, only the last N lines are returned.
    #[serde(default)]
    pub lines: Option<u32>,
}

/// Body of [`AttachInfo`].
#[derive(Debug, Deserialize, Serialize)]
pub struct AttachRequest {
    /// Seat name.
    pub seat: String,
}

/// Body of [`RigDown`]. An absent rig stops every rig.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct DownRequest {
    /// Rig name.
    #[serde(default)]
    pub rig: Option<String>,
}

/// `GET` or `POST /api/queue`.
#[derive(Debug)]
pub struct QueueCollection;

impl Endpoint for QueueCollection {
    const METHOD: &'static str = "GET";
    const PATH: &'static str = "/api/queue";
}

/// `POST /api/queue/show`.
#[derive(Debug)]
pub struct QueueShow;

impl Endpoint for QueueShow {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/queue/show";
}

/// `POST /api/queue/claim`.
#[derive(Debug)]
pub struct QueueClaim;

impl Endpoint for QueueClaim {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/queue/claim";
}

/// `POST /api/queue/handoff`.
#[derive(Debug)]
pub struct QueueHandoff;

impl Endpoint for QueueHandoff {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/queue/handoff";
}

/// `POST /api/queue/fallback`.
#[derive(Debug)]
pub struct QueueFallback;

impl Endpoint for QueueFallback {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/queue/fallback";
}

/// `POST /api/queue/unclaim`.
#[derive(Debug)]
pub struct UnclaimItem;

impl Endpoint for UnclaimItem {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/queue/unclaim";
}

/// `POST /api/send`.
#[derive(Debug)]
pub struct SendMessage;

impl Endpoint for SendMessage {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/send";
}

/// `POST /api/broadcast`.
#[derive(Debug)]
pub struct BroadcastSend;

impl Endpoint for BroadcastSend {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/broadcast";
}

/// `POST /api/ask`.
#[derive(Debug)]
pub struct AskSeat;

impl Endpoint for AskSeat {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/ask";
}

/// `GET /api/whoami`.
#[derive(Debug)]
pub struct Whoami;

impl Endpoint for Whoami {
    const METHOD: &'static str = "GET";
    const PATH: &'static str = "/api/whoami";
}

/// `POST /api/compaction/trigger`.
#[derive(Debug)]
pub struct CompactTrigger;

impl Endpoint for CompactTrigger {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/compaction/trigger";
}

/// `POST /api/compaction/cancel`.
#[derive(Debug)]
pub struct CompactCancel;

impl Endpoint for CompactCancel {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/compaction/cancel";
}

/// `POST /api/compaction/status`.
#[derive(Debug)]
pub struct CompactStatus;

impl Endpoint for CompactStatus {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/compaction/status";
}

/// Path for `POST /api/seats/{key}/hooks`.
#[must_use]
pub fn seat_hook_path(seat_key: &str) -> String {
    format!("/api/seats/{seat_key}/hooks")
}
