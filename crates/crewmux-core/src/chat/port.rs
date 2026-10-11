//! Chat persistence port. The store implements this on its writer connection.

use crewmux_types::{RigId, TopicId};

use super::error::ChatError;
use super::model::{ChatMessage, HistoryPage, HistoryQuery, NewChat, NewTopic};

/// Per-rig chat log.
///
/// `post` and `set_topic` run inside the store's writer transaction. `history`
/// and `has_topic` are reads on that same connection.
pub trait ChatRepo {
    /// Store one message.
    ///
    /// # Errors
    ///
    /// Returns [`ChatError::UnknownRig`] when the rig is missing, or [`ChatError::Store`].
    fn post(&self, message: NewChat) -> Result<ChatMessage, ChatError>;

    /// Chronological page for `query`.
    ///
    /// # Errors
    ///
    /// Returns [`ChatError::UnknownTopic`] when `query.topic` has no marker,
    /// or [`ChatError::Store`].
    fn history(&self, query: &HistoryQuery) -> Result<HistoryPage, ChatError>;

    /// Insert a topic marker and return it. The marker's [`ChatMessage::topic`] is the id.
    ///
    /// # Errors
    ///
    /// Returns [`ChatError::UnknownRig`] when the rig is missing, or [`ChatError::Store`].
    fn set_topic(&self, marker: NewTopic) -> Result<ChatMessage, ChatError>;

    /// Whether a topic marker named `name` exists for `rig`.
    ///
    /// # Errors
    ///
    /// Returns [`ChatError::Store`] on persistence failure.
    fn has_topic(&self, rig: &RigId, name: &TopicId) -> Result<bool, ChatError>;

    /// Delete every chat row for `rig`.
    ///
    /// # Errors
    ///
    /// Returns [`ChatError::Store`] on persistence failure.
    fn clear(&self, rig: &RigId) -> Result<u64, ChatError>;
}
