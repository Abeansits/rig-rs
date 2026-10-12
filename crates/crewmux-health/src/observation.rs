// Derived from openrig packages/daemon/src/domain/health-detectors.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Observations a [`crate::HealthSource`] returns. The projection decides findings.

use crewmux_core::Timestamp;
use crewmux_messaging::SessionRef;
use serde::{Deserialize, Serialize};

/// One fact a source evaluated. Absence of an observation is not a healthy verdict.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum HealthObservation {
    /// Queue coordination for one lineage.
    CoordinationLineage {
        /// Lineage id, usually a queue item.
        lineage_id: String,
        /// Coordination transitions in the window.
        coordination_transitions: u32,
        /// Product-state changes. `None` means the census is unavailable.
        product_state_changes: Option<u32>,
        /// The lineage is inside a bounded authority.
        bounded_authority: bool,
        /// Review returns with no new candidate.
        review_returns: u32,
        /// Candidate changes in the window.
        candidate_changes: u32,
        /// Newly recorded risk classes.
        new_risk_classes: u32,
        /// When the source observed it.
        at: Timestamp,
    },
    /// Wake ladder notes for one lineage.
    WakeLineage {
        /// Lineage id.
        lineage_id: String,
        /// A wake attempt was recorded as failed.
        failed: bool,
        /// An escalation rung was recorded.
        escalated: bool,
        /// Wake attempts, including rescues.
        wake_count: u32,
        /// Liveness rescues subtracted from `wake_count`.
        rescue_wake_count: u32,
        /// A next action was already recorded.
        existing_next_action: bool,
        /// When the source observed it.
        at: Timestamp,
    },
    /// A directive compared with current structured state.
    Directive {
        /// Directive id.
        directive_id: String,
        /// Declared phase, when both sides are known.
        declared_phase: Option<String>,
        /// Current phase.
        current_phase: Option<String>,
        /// Declared rigor.
        declared_rigor: Option<String>,
        /// Current rigor.
        current_rigor: Option<String>,
        /// Address of the newer state. Required for a conflict.
        conflict_source_address: Option<String>,
        /// When the source observed it.
        at: Timestamp,
    },
    /// Admission of a buildable slice.
    ScopeAdmission {
        /// Slice id.
        slice_id: String,
        /// The mission is active.
        mission_active: bool,
        /// The slice is buildable.
        buildable: bool,
        /// Authority the mission requires.
        required_authority: Option<String>,
        /// Authority recorded on the admission.
        admission_authority: Option<String>,
        /// `present`, `missing`, `contradictory`, or `unavailable`.
        admission_state: String,
        /// Where the rule was read.
        authority_source_address: Option<String>,
        /// When the source observed it.
        at: Timestamp,
    },
    /// One seat's context sample.
    ContextPressure {
        /// Seat session.
        seat: SessionRef,
        /// Tokens in use.
        used_tokens: u64,
        /// Context window. Zero is unavailable, not a zero-percent reading.
        limit: u64,
        /// When the sample was taken.
        at: Timestamp,
    },
}
