//! Health, lease, classification, and compaction failures.

use thiserror::Error;

/// A detector, policy, lease, classification, or compaction failure.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    /// The policy document cannot be trusted, so the query stops.
    #[error("health policy: {0}")]
    Policy(String),
    /// A caller passed a value this crate rejects.
    #[error("{0}")]
    Rejected(String),
    /// `limit` is outside 1..=200.
    #[error("limit must be an integer from 1 to 200")]
    Limit,
    /// Another session holds the one active classifier lease.
    #[error("lease_held: held by {holder}")]
    LeaseHeld {
        /// Session that currently holds the lease.
        holder: String,
    },
    /// Heartbeat or classify named a different session than the holder.
    #[error("lease_session_mismatch")]
    LeaseMismatch,
    /// The lease is expired or reclaimed.
    #[error("lease_not_active")]
    LeaseNotActive,
    /// The TTL has passed. Heartbeat does not revive it.
    #[error("lease_expired")]
    LeaseExpired,
    /// Reclaim or classify found no active lease.
    #[error("no_active_lease")]
    NoActiveLease,
    /// The lease id is not in the table.
    #[error("lease_not_found")]
    LeaseNotFound,
    /// `--if-dead` refused because the holder is still alive.
    #[error("lease_still_active")]
    LeaseStillActive,
    /// A classification decision failed validation.
    #[error("{0}")]
    Classify(String),
    /// Hook stdin was not a JSON value.
    #[error("hook json: {0}")]
    Hook(String),
    /// A marker or prompt could not be encoded.
    #[error("{0}")]
    Message(String),
}
