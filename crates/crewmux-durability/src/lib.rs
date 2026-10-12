//! Snapshots, checkpoints, restore, crash-cart and compaction control.

mod check;
mod conductor;
mod error;
mod manifest;
mod occupant;
mod outcome;
mod posture;
mod reconcile;
mod truth;
mod wave;

pub use check::{
    ConfidenceBreakdown, ConfidenceClass, Remediation, RestoreCheckReport, SeatCheck, SeatSignal,
    mutating_actions, restore_check,
};
pub use conductor::{CartNext, CartRig, CartWalk, CrashCartRun, conduct, kernel_first};
pub use error::Error;
pub use manifest::{SeatSnapshot, SnapshotManifest, StoredResume, sha256_hex, verify_snapshot};
pub use occupant::{OccupantResolution, OccupantRow, OccupantState, resolve_active_occupant};
pub use outcome::{RigRestoreOutcome, SeatRestoreOutcome, rollup};
pub use posture::{clamp_posture, parse_posture, posture_token};
pub use reconcile::{
    DeliveryChannel, InFlightDelivery, ReconcileAction, plan_reconcile, renudge_text,
};
pub use truth::{RuntimeEvidence, TruthReconcile, reconcile_runtime_truth};
pub use wave::{LaunchEdge, launch_waves};

use crewmux_transcript::TranscriptProbe;
use crewmux_types::SeatId;

/// Launch waves plus the delivery plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestorePlan {
    /// Waves. Only `delegates_to` and `spawned_by` constrain membership.
    pub order: Vec<Vec<SeatId>>,
    /// Pointer nudges, acks, re-sends, and escalations.
    pub reconcile: Vec<ReconcileAction>,
}

/// Build a [`RestorePlan`].
///
/// # Errors
///
/// Returns [`Error::Cycle`] or [`Error::Probe`].
pub fn restore_plan(
    seats: &[(SeatId, &str)],
    edges: &[LaunchEdge],
    inflight: &[InFlightDelivery],
    probe: &dyn TranscriptProbe,
) -> Result<RestorePlan, Error> {
    Ok(RestorePlan {
        order: launch_waves(seats, edges)?,
        reconcile: plan_reconcile(inflight, probe)?,
    })
}
