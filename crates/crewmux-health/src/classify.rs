// Derived from openrig packages/daemon/src/domain/stream-classification-worker.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Classification decisions. The daemon stores them. It does not call a model.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::error::Error;

/// Largest eligible page the attempt ledger will return.
pub const MAX_ELIGIBLE_PAGE: u32 = 100;

/// Label fields a worker may set. Values must come from the candidate set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LabelField {
    /// Item type.
    ClassificationType,
    /// Urgency.
    ClassificationUrgency,
    /// Maturity.
    ClassificationMaturity,
    /// Confidence.
    ClassificationConfidence,
    /// Destination.
    ClassificationDestination,
    /// Area.
    Area,
    /// Scope reference.
    ScopeRef,
}

impl LabelField {
    /// Every field.
    pub const ALL: [Self; 7] = [
        Self::ClassificationType,
        Self::ClassificationUrgency,
        Self::ClassificationMaturity,
        Self::ClassificationConfidence,
        Self::ClassificationDestination,
        Self::Area,
        Self::ScopeRef,
    ];
}

/// Labels on a classified item.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerLabels {
    /// Candidate-set version these values were chosen from.
    pub version: String,
    /// Field values. Each one must be in that version's candidate list.
    pub fields: BTreeMap<LabelField, String>,
    /// Whether a human is needed. `None` leaves it unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_human: Option<bool>,
    /// Duplicate of another stream item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplicate_of: Option<String>,
    /// Evidence ref for `duplicate_of`, taken from the supplied candidates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplicate_evidence: Option<String>,
}

/// One duplicate candidate the worker was shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateCandidate {
    /// Stream item id.
    pub stream_item_id: String,
    /// Positive evidence ref.
    pub evidence_ref: String,
}

/// The candidate set bound to one version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateSet {
    /// Version the labels must cite.
    pub version: String,
    /// Allowed values per field.
    pub values: BTreeMap<LabelField, BTreeSet<String>>,
    /// Duplicates the worker may name.
    pub duplicate_candidates: Vec<DuplicateCandidate>,
}

/// What the seat holding the lease decided.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ClassificationDecision {
    /// The seat declined to label the item.
    Abstain {
        /// Why. Empty is rejected.
        reason: String,
    },
    /// Labels drawn from [`CandidateSet`].
    Classify {
        /// The labels.
        labels: WorkerLabels,
    },
}

/// Check a decision against `candidates`.
///
/// # Errors
///
/// Returns [`Error::Classify`] when a label is outside the set, the version
/// disagrees, `duplicateOf` has no supplied evidence, or an abstain has no reason.
pub fn validate_decision(
    candidates: &CandidateSet,
    decision: &ClassificationDecision,
) -> Result<(), Error> {
    match decision {
        ClassificationDecision::Abstain { reason } => {
            if reason.trim().is_empty() {
                return Err(Error::Classify("abstain requires a reason".to_owned()));
            }
            Ok(())
        }
        ClassificationDecision::Classify { labels } => validate_labels(candidates, labels),
    }
}

fn validate_labels(candidates: &CandidateSet, labels: &WorkerLabels) -> Result<(), Error> {
    if labels.version != candidates.version {
        return Err(Error::Classify(format!(
            "labels version {} is not candidate set {}",
            labels.version, candidates.version
        )));
    }
    for (field, value) in &labels.fields {
        let allowed = candidates.values.get(field);
        let known = allowed.is_some_and(|set| set.contains(value));
        if !known {
            return Err(Error::Classify(format!("unavailable candidate: {value}")));
        }
    }
    validate_duplicate(candidates, labels)
}

fn validate_duplicate(candidates: &CandidateSet, labels: &WorkerLabels) -> Result<(), Error> {
    let Some(item) = labels.duplicate_of.as_deref() else {
        return Ok(());
    };
    let evidence = labels.duplicate_evidence.as_deref().unwrap_or("");
    let matched = candidates.duplicate_candidates.iter().any(|candidate| {
        candidate.stream_item_id == item
            && candidate.evidence_ref == evidence
            && !evidence.trim().is_empty()
    });
    if matched {
        Ok(())
    } else {
        Err(Error::Classify(
            "duplicate requires a positive supplied candidate and evidence reference".to_owned(),
        ))
    }
}

/// Reject a page larger than [`MAX_ELIGIBLE_PAGE`].
///
/// # Errors
///
/// Returns [`Error::Classify`] when `limit` is zero or above the cap.
pub fn cap_page(limit: u32) -> Result<u32, Error> {
    if limit == 0 || limit > MAX_ELIGIBLE_PAGE {
        Err(Error::Classify(format!(
            "eligible page must be 1..={MAX_ELIGIBLE_PAGE}"
        )))
    } else {
        Ok(limit)
    }
}
