//! Read-only view of user turns after a delivery cursor.
//!
//! Restore planning calls this. Adapters and tests supply the seat's file.

use crewmux_types::{RuntimeKind, SeatId};

use crate::error::Error;
use crate::source::{TranscriptCursor, UserTurn};

/// User turns recorded for one seat after a cursor.
pub trait TranscriptProbe {
    /// Runtime that stored the turns. Stub identity is the default.
    fn runtime(&self, seat: &SeatId) -> RuntimeKind;

    /// User turns whose bytes start at or after `after`.
    ///
    /// `None` means the whole transcript.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the transcript cannot be read.
    fn user_turns_after(
        &self,
        seat: &SeatId,
        after: Option<TranscriptCursor>,
    ) -> Result<Vec<UserTurn>, Error>;
}
