// Derived from openrig packages/daemon/src/domain/health-detectors.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Turn one observation into zero or more findings.

use crewmux_core::Timestamp;
use serde_json::{Value, json};

mod context;

use crate::detector::DetectorId;
use crate::observation::HealthObservation;
use crate::policy::HealthPolicy;
use crate::record::{HealthRecord, Severity, Status, finding_id, instance_scope};

pub(crate) fn evaluate(
    observation: &HealthObservation,
    policy: &HealthPolicy,
) -> Vec<HealthRecord> {
    match observation {
        HealthObservation::CoordinationLineage { .. } => coordination(observation, policy),
        HealthObservation::WakeLineage { .. } => wake(observation, policy),
        HealthObservation::Directive { .. } => directive(observation),
        HealthObservation::ScopeAdmission { .. } => admission(observation),
        HealthObservation::ContextPressure { .. } => context::context(observation, policy),
    }
}

struct Draft {
    detector: DetectorId,
    category: &'static str,
    severity: Severity,
    status: Status,
    subject: String,
    summary: String,
    threshold: String,
    explanation: String,
    scope: Value,
    evidence: Vec<Value>,
    at: Timestamp,
}

impl Draft {
    fn finish(self) -> HealthRecord {
        let suggested_inspection = format!("Inspect {}.", self.subject);
        HealthRecord {
            schema: crate::record::HEALTH_RECORD_SCHEMA,
            id: finding_id(self.detector, &self.subject, self.at),
            detector: self.detector,
            category: self.category,
            severity: self.severity,
            status: self.status,
            subject: self.subject,
            summary: self.summary,
            scope: self.scope,
            evidence: self.evidence,
            threshold: self.threshold,
            explanation: self.explanation,
            suggested_inspection,
            coverage: None,
        }
    }
}

fn coordination(observation: &HealthObservation, policy: &HealthPolicy) -> Vec<HealthRecord> {
    let HealthObservation::CoordinationLineage {
        lineage_id,
        coordination_transitions,
        product_state_changes,
        bounded_authority,
        review_returns,
        candidate_changes,
        new_risk_classes,
        at,
    } = observation
    else {
        return Vec::new();
    };
    let mut records = Vec::new();
    if let Some(record) = ceremony(
        lineage_id,
        *coordination_transitions,
        *product_state_changes,
        *bounded_authority,
        *at,
        policy,
    ) {
        records.push(record);
    }
    if *review_returns >= policy.thresholds.review_returns
        && *candidate_changes == 0
        && *new_risk_classes == 0
    {
        records.push(review(lineage_id, *review_returns, *at, policy));
    }
    records
}

fn ceremony(
    lineage_id: &str,
    transitions: u32,
    product: Option<u32>,
    bounded: bool,
    at: Timestamp,
    policy: &HealthPolicy,
) -> Option<HealthRecord> {
    let gate = policy.thresholds.ceremony_transitions;
    let ratio_gate = policy.thresholds.ceremony_ratio;
    if !ceremony_fires(transitions, product, bounded, gate, ratio_gate) {
        return None;
    }
    let (status, summary, explanation) = ceremony_text(lineage_id, transitions, product);
    Some(
        Draft {
            detector: DetectorId::CeremonyAmplification,
            category: "process",
            severity: Severity::Warning,
            status,
            subject: lineage_id.to_owned(),
            summary,
            threshold: format!(
                "coordinationTransitions >= {gate} AND coordinationTransitions / max(productStateChanges, 1) >= {ratio_gate} AND boundedAuthority = false"
            ),
            explanation,
            scope: instance_scope(lineage_id),
            evidence: vec![json!({"type": "queue-transition", "qitemId": lineage_id})],
            at,
        }
        .finish(),
    )
}

fn ceremony_fires(
    transitions: u32,
    product: Option<u32>,
    bounded: bool,
    gate: u32,
    ratio_gate: u32,
) -> bool {
    match product {
        None => transitions >= gate,
        Some(changes) => {
            !bounded && transitions >= gate && transitions / changes.max(1) >= ratio_gate
        }
    }
}

fn ceremony_text(
    lineage_id: &str,
    transitions: u32,
    product: Option<u32>,
) -> (Status, String, String) {
    match product {
        None => (
            Status::Indeterminate,
            format!("Ceremony proportionality is indeterminate for {lineage_id}."),
            format!(
                "{transitions} coordination transitions in one lineage; product-outcome census unavailable, so no ratio is computed."
            ),
        ),
        Some(changes) => {
            let ratio = transitions / changes.max(1);
            (
                Status::Active,
                format!("Coordination activity is disproportionate for {lineage_id}."),
                format!(
                    "{transitions} coordination transitions for {changes} product-state changes ({ratio}:1)."
                ),
            )
        }
    }
}

fn review(
    lineage_id: &str,
    review_returns: u32,
    at: Timestamp,
    policy: &HealthPolicy,
) -> HealthRecord {
    Draft {
        detector: DetectorId::ReviewCarousel,
        category: "process",
        severity: Severity::Warning,
        status: Status::Active,
        subject: lineage_id.to_owned(),
        summary: format!("Review repeatedly returned the unchanged {lineage_id} lineage."),
        threshold: format!(
            "reviewReturns >= {} AND candidateChanges = 0 AND newRiskClasses = 0",
            policy.thresholds.review_returns
        ),
        explanation: format!("{review_returns} review returns occurred with no candidate change."),
        scope: instance_scope(lineage_id),
        evidence: vec![json!({"type": "queue-transition", "qitemId": lineage_id})],
        at,
    }
    .finish()
}

fn wake(observation: &HealthObservation, policy: &HealthPolicy) -> Vec<HealthRecord> {
    let HealthObservation::WakeLineage {
        lineage_id,
        failed,
        escalated,
        wake_count,
        rescue_wake_count,
        existing_next_action,
        at,
    } = observation
    else {
        return Vec::new();
    };
    if *escalated {
        return Vec::new();
    }
    if *failed {
        return vec![failed_wake(lineage_id, *at)];
    }
    let redundant = wake_count.saturating_sub(*rescue_wake_count);
    if !existing_next_action || redundant < policy.thresholds.redundant_wakes {
        return Vec::new();
    }
    vec![storm(
        lineage_id,
        *wake_count,
        *rescue_wake_count,
        redundant,
        *at,
        policy,
    )]
}

fn failed_wake(lineage_id: &str, at: Timestamp) -> HealthRecord {
    Draft {
        detector: DetectorId::WakeLineage,
        category: "process",
        severity: Severity::Warning,
        status: Status::Active,
        subject: lineage_id.to_owned(),
        summary: format!("Wake for {lineage_id} failed and was not escalated."),
        threshold: "a failed wake-attempt with no escalation-rung".to_owned(),
        explanation: "The ladder recorded outcome=failed and no escalation rung.".to_owned(),
        scope: instance_scope(lineage_id),
        evidence: vec![json!({
            "type": "queue-transition",
            "qitemId": lineage_id,
            "note": "wake-attempt outcome=failed"
        })],
        at,
    }
    .finish()
}

fn storm(
    lineage_id: &str,
    wake_count: u32,
    rescue_wake_count: u32,
    redundant: u32,
    at: Timestamp,
    policy: &HealthPolicy,
) -> HealthRecord {
    Draft {
        detector: DetectorId::RedundantWakeStorm,
        category: "process",
        severity: Severity::Warning,
        status: Status::Active,
        subject: lineage_id.to_owned(),
        summary: format!("Repeated wakes duplicated an existing next action for {lineage_id}."),
        threshold: format!(
            "wakeCount - rescueWakeCount >= {} AND existingNextAction = true",
            policy.thresholds.redundant_wakes
        ),
        explanation: format!(
            "{wake_count} wakes minus {rescue_wake_count} liveness rescues left {redundant} redundant wakes."
        ),
        scope: instance_scope(lineage_id),
        evidence: vec![json!({"type": "queue-transition", "qitemId": lineage_id})],
        at,
    }
    .finish()
}

fn directive(observation: &HealthObservation) -> Vec<HealthRecord> {
    let HealthObservation::Directive {
        directive_id,
        declared_phase,
        current_phase,
        declared_rigor,
        current_rigor,
        conflict_source_address,
        at,
    } = observation
    else {
        return Vec::new();
    };
    let Some(address) = conflict_source_address.as_deref() else {
        return Vec::new();
    };
    let phase = differs(declared_phase.as_deref(), current_phase.as_deref());
    let rigor = differs(declared_rigor.as_deref(), current_rigor.as_deref());
    if !phase && !rigor {
        return Vec::new();
    }
    vec![
        Draft {
            detector: DetectorId::StaleDirective,
            category: "governance",
            severity: Severity::Warning,
            status: Status::Active,
            subject: directive_id.clone(),
            summary: format!("Directive {directive_id} conflicts with current structured state."),
            threshold: "a declared phase or rigor differs from newer structured state".to_owned(),
            explanation: format!(
                "current state is addressed by {address}. Age alone was not used."
            ),
            scope: instance_scope(directive_id),
            evidence: vec![json!({"type": "work-graph", "nodeId": directive_id})],
            at: *at,
        }
        .finish(),
    ]
}

fn differs(left: Option<&str>, right: Option<&str>) -> bool {
    matches!((left, right), (Some(left), Some(right)) if left != right)
}

fn admission(observation: &HealthObservation) -> Vec<HealthRecord> {
    let HealthObservation::ScopeAdmission {
        slice_id,
        mission_active,
        buildable,
        required_authority,
        admission_authority,
        admission_state,
        authority_source_address,
        at,
    } = observation
    else {
        return Vec::new();
    };
    if !mission_active
        || !buildable
        || required_authority.is_none()
        || authority_source_address.is_none()
    {
        return Vec::new();
    }
    let present_match = admission_state == "present" && admission_authority == required_authority;
    if admission_state == "unavailable" || present_match {
        return Vec::new();
    }
    let drifted = admission_state == "missing"
        || admission_state == "contradictory"
        || admission_state == "present";
    if !drifted {
        return Vec::new();
    }
    vec![
        Draft {
            detector: DetectorId::ScopeAdmissionDrift,
            category: "governance",
            severity: Severity::Warning,
            status: Status::Active,
            subject: slice_id.clone(),
            summary: format!("Buildable slice {slice_id} lacks its mission's required admission."),
            threshold: "missionActive = true AND buildable = true AND admission is missing or contradictory"
                .to_owned(),
            explanation: format!("admission is {admission_state}."),
            scope: instance_scope(slice_id),
            evidence: vec![json!({"type": "work-graph", "nodeId": slice_id, "nodeType": "slice"})],
            at: *at,
        }
        .finish(),
    ]
}
