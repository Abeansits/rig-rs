//! Canonical `GET /api/events` envelope.
//!
//! Slice 13's interim stream uses the same path and the same `{id, type, payload}`
//! shape. New kinds are additive. Unknown kinds deserialize as [`StreamKind::Other`].

use serde::{Deserialize, Serialize};

/// One event on the SSE or WebSocket stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamEvent {
    /// Monotonic id. Clients resume with `Last-Event-ID` or `last_event_id`.
    pub id: u64,
    /// Discriminator.
    #[serde(rename = "type")]
    pub kind: StreamKind,
    /// Present for every kind except [`StreamKind::Gap`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<StreamPayload>,
}

/// Stream discriminators. Slice 13 names stay here so that client can adopt this type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StreamKind {
    /// A seat's lifecycle changed.
    SeatStateChanged,
    /// The pane process exited.
    SeatDied,
    /// Human or agent activity.
    Activity,
    /// The backlog dropped events the client has not seen.
    Gap,
    /// A queue row changed.
    QueueChanged,
    /// A direct send recorded a receipt.
    DeliveryRecorded,
    /// Contamination or another attention signal.
    AttentionRaised,
    /// A stream item was stored. `/api/stream/watch` follows this kind.
    StreamEmitted,
    /// A classifier lease changed.
    Classifier,
    /// A kind this build does not name. Never emitted.
    #[serde(other)]
    Other,
}

impl StreamKind {
    /// Wire token. [`StreamKind::Other`] is `other`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SeatStateChanged => "seat_state_changed",
            Self::SeatDied => "seat_died",
            Self::Activity => "activity",
            Self::Gap => "gap",
            Self::QueueChanged => "queue_changed",
            Self::DeliveryRecorded => "delivery_recorded",
            Self::AttentionRaised => "attention_raised",
            Self::StreamEmitted => "stream_emitted",
            Self::Classifier => "classifier",
            Self::Other => "other",
        }
    }
}

/// Optional fields. Absent fields are omitted so older clients ignore them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamPayload {
    /// Seat id or session address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seat: Option<String>,
    /// Seat lifecycle token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Short text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Queue item id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qitem: Option<String>,
    /// Delivery outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// Attention or defer reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Stream item id for [`StreamKind::StreamEmitted`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    /// Stream sequence, when the event names one item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    /// Pane exit status from `seat_died`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_status: Option<i32>,
    /// `pane_dead_signal` from `seat_died`, when the process was killed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<String>,
    /// Last pane capture from `seat_died`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<String>,
}

/// `GET /api/events`.
#[derive(Debug)]
pub struct EventStream;

impl crate::endpoint::Endpoint for EventStream {
    const METHOD: &'static str = "GET";
    const PATH: &'static str = "/api/events";
}
