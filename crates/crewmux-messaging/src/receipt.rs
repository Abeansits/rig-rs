//! Direct-send receipts. Slice 8 re-delivers from the fingerprint and cursor.

use crewmux_core::Timestamp;
use serde::{Deserialize, Serialize};

/// How a guarded paste ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryOutcome {
    /// Enter was sent after a clean paste.
    Delivered,
    /// The guard asked us to wait.
    Deferred,
    /// The guard gave up deferring.
    Escalated,
    /// The composer contained text beyond the payload, so Enter was not sent.
    Contaminated,
    /// The payload did not render. The composer was cleared and Enter was not sent.
    Failed,
}

/// One seat's receipt. `cursor` is the daemon's monotonic delivery counter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryReceipt {
    /// Outcome.
    pub outcome: DeliveryOutcome,
    /// True when the composer state was unknown and only the copy-mode and typing rules ran.
    pub guard_degraded: bool,
    /// Fingerprint of the addressed payload.
    pub fingerprint: String,
    /// Delivery cursor for slice 8.
    pub cursor: u64,
    /// When.
    pub at: Timestamp,
    /// Recorded `--dangerously-interact` reason, when one was required.
    pub interact_reason: Option<String>,
}
