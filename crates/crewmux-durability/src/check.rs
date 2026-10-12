//! Claude transcript check used by [`crewmux_adapters::SeatIo::submit`].

use crewmux_adapters::{DeliveryCheck, Error as AdapterError, ProbeQuery, TurnVerdict};

use crate::error::Error;
use crate::normalize::{DeliveryMatch, DeliveryRuntime, match_for};
use crate::probe::{transcript_len, transcript_path, user_turns_after};

/// Reads `~/.claude/projects/<cwd-slug>/<session-id>.jsonl` and applies ADR-0009.
#[derive(Debug, Default)]
pub struct ClaudeDelivery;

impl DeliveryCheck for ClaudeDelivery {
    fn cursor(&self, query: &ProbeQuery) -> Result<u64, AdapterError> {
        let path = transcript_path(&query.home, &query.cwd, &query.session_id);
        transcript_len(&path).map_err(map_err)
    }

    fn turns(&self, query: &ProbeQuery) -> Result<Vec<String>, AdapterError> {
        let path = transcript_path(&query.home, &query.cwd, &query.session_id);
        let turns = user_turns_after(&path, query.after_bytes).map_err(map_err)?;
        Ok(turns.into_iter().map(|turn| turn.text).collect())
    }

    fn verdict(&self, sent: &str, stored: &str) -> TurnVerdict {
        match match_for(DeliveryRuntime::Claude, sent, stored) {
            DeliveryMatch::Equal => TurnVerdict::Equal,
            DeliveryMatch::Different => TurnVerdict::Different,
            DeliveryMatch::Composite => TurnVerdict::Composite,
        }
    }
}

fn map_err(err: Error) -> AdapterError {
    match err {
        Error::Io(err) => AdapterError::Io(err),
        Error::Parse { .. } => AdapterError::Message(err.to_string()),
    }
}
