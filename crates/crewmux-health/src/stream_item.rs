// Derived from openrig packages/daemon/src/domain/stream-store.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! One item on the operator stream.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A stored stream item. The daemon publishes it on `/api/events` when it is new.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamItem {
    /// Caller-supplied or minted id.
    pub id: String,
    /// Item kind. `note` when the caller omitted one.
    pub kind: String,
    /// Body text.
    pub body: String,
    /// Labels. An unclassified item stores `{}`.
    pub labels: Value,
    /// Archived items leave the default list.
    pub archived: bool,
    /// Session that emitted the item.
    pub source: String,
    /// Emit time, milliseconds.
    pub emitted_at_ms: i64,
    /// Chronological key.
    pub seq: i64,
}
