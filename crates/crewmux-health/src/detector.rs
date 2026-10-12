// Derived from openrig packages/daemon/src/domain/health-detectors.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Detector ids. An omitted detector was not evaluated.

use serde::{Deserialize, Serialize};

/// One health detector. Wire names match the TypeScript tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DetectorId {
    /// Coordination without a product outcome.
    #[serde(rename = "process.ceremony-amplification")]
    CeremonyAmplification,
    /// Review returned an unchanged candidate.
    #[serde(rename = "process.review-carousel")]
    ReviewCarousel,
    /// Wakes that repeat an existing next action.
    #[serde(rename = "process.redundant-wake-storm")]
    RedundantWakeStorm,
    /// A failed wake that the ladder never escalated.
    #[serde(rename = "process.wake-lineage")]
    WakeLineage,
    /// A directive that disagrees with newer structured state.
    #[serde(rename = "governance.stale-directive")]
    StaleDirective,
    /// A buildable slice missing its admission.
    #[serde(rename = "governance.scope-admission-drift")]
    ScopeAdmissionDrift,
    /// Seat context above the policy percent.
    #[serde(rename = "context.pressure")]
    ContextPressure,
}

impl DetectorId {
    /// Every detector this build knows.
    pub const ALL: [Self; 7] = [
        Self::CeremonyAmplification,
        Self::ReviewCarousel,
        Self::RedundantWakeStorm,
        Self::WakeLineage,
        Self::StaleDirective,
        Self::ScopeAdmissionDrift,
        Self::ContextPressure,
    ];

    /// Wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CeremonyAmplification => "process.ceremony-amplification",
            Self::ReviewCarousel => "process.review-carousel",
            Self::RedundantWakeStorm => "process.redundant-wake-storm",
            Self::WakeLineage => "process.wake-lineage",
            Self::StaleDirective => "governance.stale-directive",
            Self::ScopeAdmissionDrift => "governance.scope-admission-drift",
            Self::ContextPressure => "context.pressure",
        }
    }
}
