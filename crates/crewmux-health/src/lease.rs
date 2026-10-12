// Derived from openrig packages/daemon/src/domain/classifier-lease-manager.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Classifier lease. The database allows one active row. Reclaim is operator-only.

use crewmux_messaging::SessionRef;
use serde::{Deserialize, Serialize};

/// `active`, `expired`, or `reclaimed` (the operator release).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LeaseState {
    /// The holder may classify.
    Active,
    /// Evaluation marked the TTL stale. This is not a reclaim.
    Expired,
    /// An operator verb took the lease. The daemon never does this on its own.
    #[serde(rename = "reclaimed")]
    Released,
}

/// One classifier lease.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassifierLease {
    /// Lease id.
    pub id: String,
    /// Holder.
    pub holder: SessionRef,
    /// Acquire time, milliseconds.
    pub acquired_at_ms: i64,
    /// Expiry time, milliseconds.
    pub expires_at_ms: i64,
    /// Last heartbeat, milliseconds.
    pub last_heartbeat_ms: i64,
    /// Lifecycle.
    pub state: LeaseState,
}

/// Default TTL: 15 minutes.
pub const DEFAULT_LEASE_TTL_MS: i64 = 15 * 60 * 1000;

/// A source whose tables are not in this schema. It reports coverage `none`.
#[derive(Clone, Copy, Debug)]
pub struct AbsentSource {
    /// Coverage name.
    pub name: &'static str,
    /// Detectors that would run if the tables existed.
    pub detectors: &'static [crate::detector::DetectorId],
}

impl crate::project::HealthSource for AbsentSource {
    fn name(&self) -> &str {
        self.name
    }

    fn detectors(&self) -> &[crate::detector::DetectorId] {
        self.detectors
    }

    fn read(
        &self,
        _query: &crate::project::HealthQuery,
    ) -> Result<
        (
            Vec<crate::observation::HealthObservation>,
            Option<crate::coverage::Coverage>,
        ),
        crate::error::Error,
    > {
        Ok((Vec::new(), None))
    }
}
