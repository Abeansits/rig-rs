//! Chat ids, topic names, cursors, and sender identities.

/// Seat id stamped into a seat process and sent as [`SEAT_ID_HEADER`].
pub const SEAT_ID_ENV: &str = "RIG_SEAT_ID";

/// Secret that proves [`SEAT_ID_ENV`]. A wrong value is not a sender.
pub const SEAT_TOKEN_ENV: &str = "RIG_SEAT_TOKEN";

/// HTTP header carrying [`SEAT_ID_ENV`].
pub const SEAT_ID_HEADER: &str = "X-Rig-Seat-Id";

/// HTTP header carrying [`SEAT_TOKEN_ENV`].
pub const SEAT_TOKEN_HEADER: &str = "X-Rig-Seat-Token";

/// Longest topic name, in bytes.
pub const MAX_TOPIC_LEN: usize = 256;

/// Longest sender claim or stored sender, in bytes.
pub const MAX_SENDER_LEN: usize = 256;

/// Longest history cursor, in bytes. Message ids are 26-character ULIDs.
pub const MAX_CURSOR_LEN: usize = 256;

/// Longest presented seat token, in bytes. Issued tokens are ULIDs.
pub const MAX_SEAT_TOKEN_LEN: usize = 256;

const _: () = assert!(MAX_SENDER_LEN == crate::id::MAX_NAME_LEN);

use std::fmt;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::error::TypeError;

/// Monotonic chat-message identifier. Values increase across calls in this process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChatId(Ulid);

impl ChatId {
    /// Next id. Later calls sort after earlier ones, including inside one millisecond.
    #[must_use]
    pub fn generate() -> Self {
        let fresh = Ulid::generate();
        Self(with_last(|prev| bump(prev, fresh)))
    }

    /// Wrap an existing ULID.
    #[must_use]
    pub const fn from_ulid(id: Ulid) -> Self {
        Self(id)
    }

    /// The underlying ULID.
    #[must_use]
    pub const fn as_ulid(self) -> Ulid {
        self.0
    }
}

impl fmt::Display for ChatId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for ChatId {
    type Err = TypeError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Ulid::from_str(text)
            .map(Self)
            .map_err(|err| TypeError::Id(err.to_string()))
    }
}

fn with_last(update: impl FnOnce(u128) -> Ulid) -> Ulid {
    static LOCK: AtomicBool = AtomicBool::new(false);
    static HI: AtomicU64 = AtomicU64::new(0);
    static LO: AtomicU64 = AtomicU64::new(0);
    while LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        std::hint::spin_loop();
    }
    let prev = join(HI.load(Ordering::Relaxed), LO.load(Ordering::Relaxed));
    let next = update(prev);
    let (hi, lo) = split(next.0);
    HI.store(hi, Ordering::Relaxed);
    LO.store(lo, Ordering::Relaxed);
    LOCK.store(false, Ordering::Release);
    next
}

fn join(hi: u64, lo: u64) -> u128 {
    (u128::from(hi) << 64) | u128::from(lo)
}

fn split(value: u128) -> (u64, u64) {
    let hi = u64::try_from(value >> 64).unwrap_or(0);
    let lo = u64::try_from(value & 0xffff_ffff_ffff_ffff).unwrap_or(0);
    (hi, lo)
}

fn bump(prev: u128, fresh: Ulid) -> Ulid {
    if prev == 0 {
        return fresh;
    }
    let previous = Ulid(prev);
    if fresh > previous {
        return fresh;
    }
    let max_rand = (1_u128 << 80) - 1;
    let bumped = previous.random().saturating_add(1);
    if bumped == 0 || bumped > max_rand {
        Ulid::from_parts(previous.timestamp_ms().saturating_add(1), 1)
    } else {
        Ulid::from_parts(previous.timestamp_ms(), bumped)
    }
}

/// Topic name stored on a marker row.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TopicId(String);

impl TopicId {
    /// Reject an empty name, a name past [`MAX_TOPIC_LEN`] bytes, or a control character.
    ///
    /// # Errors
    ///
    /// Returns [`TypeError::Name`] when `text` is empty, longer than [`MAX_TOPIC_LEN`],
    /// or contains a control character.
    pub fn new(text: impl Into<String>) -> Result<Self, TypeError> {
        bounded_label(text.into(), MAX_TOPIC_LEN).map(Self)
    }

    /// Borrow the name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TopicId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Opaque forward cursor. Compared as text against message ids.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChatCursor(String);

impl ChatCursor {
    /// Reject an empty cursor, one past [`MAX_CURSOR_LEN`] bytes, or a control character.
    ///
    /// # Errors
    ///
    /// Returns [`TypeError::Name`] when `text` is empty, longer than [`MAX_CURSOR_LEN`],
    /// or contains a control character.
    pub fn new(text: impl Into<String>) -> Result<Self, TypeError> {
        bounded_label(text.into(), MAX_CURSOR_LEN).map(Self)
    }

    /// Cursor positioned on `id`.
    #[must_use]
    pub fn from_chat_id(id: ChatId) -> Self {
        Self(id.to_string())
    }

    /// Borrow the cursor text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ChatCursor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Who wrote a chat row. Outside a seat this is [`Self::OPERATOR`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SenderRef(String);

impl SenderRef {
    /// Attribution for a call that did not present seat credentials.
    pub const OPERATOR: &'static str = "operator";

    /// The operator identity.
    #[must_use]
    pub fn operator() -> Self {
        Self(Self::OPERATOR.to_owned())
    }

    /// The seat whose token matched.
    #[must_use]
    pub fn seat(id: crate::SeatId) -> Self {
        Self(id.to_string())
    }

    /// Reject an empty sender, one past [`MAX_SENDER_LEN`] bytes, or a control character.
    ///
    /// # Errors
    ///
    /// Returns [`TypeError::Name`] when `text` is empty, longer than [`MAX_SENDER_LEN`],
    /// or contains a control character.
    pub fn parse(text: &str) -> Result<Self, TypeError> {
        bounded_label(text.to_owned(), MAX_SENDER_LEN).map(Self)
    }

    /// Borrow the sender text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SenderRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

fn bounded_label(text: String, max: usize) -> Result<String, TypeError> {
    if text.is_empty() || text.len() > max || text.chars().any(char::is_control) {
        return Err(TypeError::Name(text));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::{ChatCursor, MAX_CURSOR_LEN, MAX_SENDER_LEN, MAX_TOPIC_LEN, SenderRef, TopicId};

    #[test]
    fn topic_accepts_256_bytes_and_rejects_257() {
        let ok = "\"".repeat(MAX_TOPIC_LEN);
        assert_eq!(TopicId::new(&ok).expect("topic").as_str(), ok);
        assert!(TopicId::new("\"".repeat(MAX_TOPIC_LEN.saturating_add(1))).is_err());
        assert!(TopicId::new("").is_err());
        assert!(TopicId::new("a\nb").is_err());
    }

    #[test]
    fn sender_and_cursor_reject_overlong_text() {
        let ok = "a".repeat(MAX_SENDER_LEN);
        assert_eq!(SenderRef::parse(&ok).expect("sender").as_str(), ok);
        assert!(SenderRef::parse(&"a".repeat(MAX_SENDER_LEN.saturating_add(1))).is_err());
        let cursor = "b".repeat(MAX_CURSOR_LEN);
        assert_eq!(ChatCursor::new(&cursor).expect("cursor").as_str(), cursor);
        assert!(ChatCursor::new("b".repeat(MAX_CURSOR_LEN.saturating_add(1))).is_err());
    }
}
