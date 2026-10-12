//! What [`crate::SeatIo::submit`] asks the transcript probe.

use std::path::PathBuf;

use crate::error::Error;

/// Where to read user turns that landed after a paste.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeQuery {
    /// Home directory that holds `.claude`.
    pub home: PathBuf,
    /// Seat working directory. Claude slugs this into the project path.
    pub cwd: PathBuf,
    /// Claude `--session-id` / `--resume` value.
    pub session_id: String,
    /// Transcript length recorded before the paste.
    pub after_bytes: u64,
}

/// Result of comparing a sent body with one stored user turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnVerdict {
    /// The normalised texts are the same delivery.
    Equal,
    /// The texts differ.
    Different,
    /// The turn contains more than one paste wrapper.
    Composite,
}

/// Transcript reads and the ADR-0009 comparison.
///
/// The Claude jsonl reader lives in `crewmux-durability`. The adapter calls
/// this trait so the two crates do not form a cycle.
pub trait DeliveryCheck: Send + Sync {
    /// Transcript length before the paste. Missing files are `0`.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the file exists but cannot be stated.
    fn cursor(&self, query: &ProbeQuery) -> Result<u64, Error>;

    /// User-turn texts recorded after the delivery cursor.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the transcript cannot be read.
    fn turns(&self, query: &ProbeQuery) -> Result<Vec<String>, Error>;

    /// Compare `sent` with one stored turn.
    fn verdict(&self, sent: &str, stored: &str) -> TurnVerdict;
}
