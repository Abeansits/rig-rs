//! Chat ids, topic names, cursors, and sender identities.

/// Seat id stamped into a seat process and sent as [`SEAT_ID_HEADER`].
pub const SEAT_ID_ENV: &str = "RIG_SEAT_ID";

/// Secret that proves [`SEAT_ID_ENV`]. A wrong value is not a sender.
pub const SEAT_TOKEN_ENV: &str = "RIG_SEAT_TOKEN";

/// HTTP header carrying [`SEAT_ID_ENV`].
pub const SEAT_ID_HEADER: &str = "X-Rig-Seat-Id";

/// HTTP header carrying [`SEAT_TOKEN_ENV`].
pub const SEAT_TOKEN_HEADER: &str = "X-Rig-Seat-Token";

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
    /// Reject an empty name or a name that contains a control character.
    ///
    /// # Errors
    ///
    /// Returns [`TypeError::Name`] when `text` is empty or contains a control character.
    pub fn new(text: impl Into<String>) -> Result<Self, TypeError> {
        let text = text.into();
        if text.is_empty() || text.chars().any(char::is_control) {
            return Err(TypeError::Name(text));
        }
        Ok(Self(text))
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
    /// Reject an empty cursor or one that contains a control character.
    ///
    /// # Errors
    ///
    /// Returns [`TypeError::Name`] when `text` is empty or contains a control character.
    pub fn new(text: impl Into<String>) -> Result<Self, TypeError> {
        let text = text.into();
        if text.is_empty() || text.chars().any(char::is_control) {
            return Err(TypeError::Name(text));
        }
        Ok(Self(text))
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

    /// Reject an empty sender or one that contains a control character.
    ///
    /// # Errors
    ///
    /// Returns [`TypeError::Name`] when `text` is empty or contains a control character.
    pub fn parse(text: &str) -> Result<Self, TypeError> {
        if text.is_empty() || text.chars().any(char::is_control) {
            return Err(TypeError::Name(text.to_owned()));
        }
        Ok(Self(text.to_owned()))
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
