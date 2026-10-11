//! Chat HTTP bodies.
//!
//! Watch stays on `GET /api/rigs/{id}/chat/watch`. The live `/api/events` tail
//! uses [`crate::StreamEvent`]: a numeric log id and a seat payload. A chat
//! frame resumes from the message id and carries the chat row, so it is
//! [`ChatWatchEvent`] on its own route until slice 4 owns the canonical stream.

use serde::{Deserialize, Serialize};

use crate::endpoint::Endpoint;

/// `POST /api/rigs/{rig_id}/chat/send`.
#[derive(Debug)]
pub struct ChatSend;

impl Endpoint for ChatSend {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/rigs/{rig_id}/chat/send";
}

/// `GET /api/rigs/{rig_id}/chat/history`.
#[derive(Debug)]
pub struct ChatHistory;

impl Endpoint for ChatHistory {
    const METHOD: &'static str = "GET";
    const PATH: &'static str = "/api/rigs/{rig_id}/chat/history";
}

/// `GET /api/rigs/{rig_id}/chat/watch`.
///
/// Temporary chat-only SSE route. Fold it into `/api/events` when that stream exists.
#[derive(Debug)]
pub struct ChatWatch;

impl Endpoint for ChatWatch {
    const METHOD: &'static str = "GET";
    const PATH: &'static str = "/api/rigs/{rig_id}/chat/watch";
}

/// `POST /api/rigs/{rig_id}/chat/topic`.
#[derive(Debug)]
pub struct ChatTopic;

impl Endpoint for ChatTopic {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/rigs/{rig_id}/chat/topic";
}

/// `POST /api/rigs/{rig_id}/chat/clear`.
#[derive(Debug)]
pub struct ChatClear;

impl Endpoint for ChatClear {
    const METHOD: &'static str = "POST";
    const PATH: &'static str = "/api/rigs/{rig_id}/chat/clear";
}

/// `GET /api/rigs/summary`.
#[derive(Debug)]
pub struct RigSummary;

impl Endpoint for RigSummary {
    const METHOD: &'static str = "GET";
    const PATH: &'static str = "/api/rigs/summary";
}

/// One rig id and name, sorted by name on the wire.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RigSummaryRow {
    /// Rig id.
    pub id: String,
    /// Rig name.
    pub name: String,
}

/// One chat row as the history and send routes return it.
///
/// Field order matches the TypeScript message object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatWire {
    /// Message id.
    pub id: String,
    /// Owning rig.
    #[serde(rename = "rigId")]
    pub rig_id: String,
    /// Seat id, or `operator`.
    pub sender: String,
    /// `message` or `topic`.
    pub kind: String,
    /// Message text.
    pub body: String,
    /// Topic name on a marker. `null` on an ordinary message.
    pub topic: Option<String>,
    /// `YYYY-MM-DD HH:MM:SS` in UTC.
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

/// Chat SSE payload `{id, type, payload}`.
///
/// `id` is the message id. The shared [`crate::StreamEvent`] uses a log sequence
/// and a seat payload, so chat does not reuse that struct.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatWatchEvent<T> {
    /// Event id. For chat this is the message id, and it is also the SSE `id:` line.
    pub id: String,
    /// Event name, such as `chat.message`.
    #[serde(rename = "type")]
    pub event_type: String,
    /// Event body.
    pub payload: T,
}

/// `chat.message`.
pub const CHAT_MESSAGE_TYPE: &str = "chat.message";
