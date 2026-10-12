//! Chat rows and history queries.

use crewmux_types::{ChatCursor, ChatId, RigId, SenderRef, TopicId};

use crate::clock::Timestamp;

/// One stored chat row. `topic` is set only on a topic marker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMessage {
    /// Message id. Later ids sort after earlier ones.
    pub id: ChatId,
    /// Owning rig.
    pub rig: RigId,
    /// Seat id, or [`SenderRef::OPERATOR`].
    pub sender: SenderRef,
    /// Message text. Empty for a topic marker that carried no body.
    pub body: String,
    /// Instant the row was stored, in milliseconds.
    pub at: Timestamp,
    /// Topic name when this row is a marker.
    pub topic: Option<TopicId>,
}

impl ChatMessage {
    /// `topic` for a marker, `message` otherwise.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        if self.topic.is_some() {
            "topic"
        } else {
            "message"
        }
    }
}

/// Filters for [`super::ChatRepo::history`].
///
/// Without `after`, the page is the newest `limit` rows in chronological order.
/// With `after`, the page continues forward from that cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryQuery {
    /// Owning rig.
    pub rig: RigId,
    /// Topic window. Unknown names are [`super::ChatError::UnknownTopic`].
    pub topic: Option<TopicId>,
    /// Return rows with ids strictly after this cursor.
    pub after: Option<ChatCursor>,
    /// Inclusive instant lower bound.
    pub since: Option<Timestamp>,
    /// Exact sender match.
    pub sender: Option<SenderRef>,
    /// Maximum rows in the page.
    pub limit: u32,
}

/// One history page, oldest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryPage {
    /// Rows in chronological order.
    pub messages: Vec<ChatMessage>,
    /// Cursor to request the following forward page, when one exists.
    pub next: Option<ChatCursor>,
}

/// A chat line to store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewChat {
    /// Owning rig.
    pub rig: RigId,
    /// Resolved sender.
    pub sender: SenderRef,
    /// Message text.
    pub body: String,
    /// Instant to store.
    pub at: Timestamp,
}

/// A topic marker to store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewTopic {
    /// Owning rig.
    pub rig: RigId,
    /// Resolved sender.
    pub sender: SenderRef,
    /// Topic name.
    pub name: TopicId,
    /// Optional body. Empty when the caller omitted it.
    pub body: String,
    /// Instant to store.
    pub at: Timestamp,
}
