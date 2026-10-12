//! Health projection behavior. Case titles in the ledger name the TypeScript tests these cover.

use std::sync::atomic::{AtomicU32, Ordering};

use crewmux_core::Timestamp;
use crewmux_health::{
    CandidateSet, ClassificationDecision, ContextFact, Coverage, DetectorId, DuplicateCandidate,
    Error, HealthObservation, HealthPolicy, HealthQuery, HealthSource, LabelField, NodeFact,
    SeatSample, TransitionNote, WorkerLabels, cap_page, context_observations, context_summary,
    node_summary, project, run_query, validate_decision, wake_from_notes,
};
use crewmux_messaging::SessionRef;
use crewmux_types::QItemId;

fn at() -> Timestamp {
    Timestamp::from_millis(1_700_000_000_000)
}

fn query(limit: u32) -> HealthQuery {
    HealthQuery {
        limit,
        now: at(),
        severity: None,
        status: None,
    }
}

#[expect(
    clippy::expect_used,
    reason = "fixture builder; the session string is a test constant"
)]
fn seat(name: &str) -> SessionRef {
    SessionRef::new(name).expect("session")
}

fn pressure(name: &str, used: u64, limit: u64) -> HealthObservation {
    HealthObservation::ContextPressure {
        seat: seat(name),
        used_tokens: used,
        limit,
        at: at(),
    }
}

enum Outcome {
    Fail(&'static str),
    Ready(Vec<HealthObservation>, Coverage),
}

struct Counting {
    name: &'static str,
    detectors: &'static [DetectorId],
    hits: AtomicU32,
    outcome: Outcome,
}

impl HealthSource for Counting {
    fn name(&self) -> &str {
        self.name
    }

    fn detectors(&self) -> &[DetectorId] {
        self.detectors
    }

    fn read(&self, _: &HealthQuery) -> Result<(Vec<HealthObservation>, Option<Coverage>), Error> {
        self.hits.fetch_add(1, Ordering::Relaxed);
        match &self.outcome {
            Outcome::Fail(reason) => Err(Error::Rejected((*reason).to_owned())),
            Outcome::Ready(observations, coverage) => Ok((observations.clone(), Some(*coverage))),
        }
    }
}

#[test]
fn failed_source_is_named_without_an_invented_count() {
    let broken = Counting {
        name: "checkpoints",
        detectors: &[DetectorId::ContextPressure],
        hits: AtomicU32::new(0),
        outcome: Outcome::Fail("broken checkpoint JSON"),
    };
    let working = Counting {
        name: "context",
        detectors: &[DetectorId::ContextPressure],
        hits: AtomicU32::new(0),
        outcome: Outcome::Ready(
            vec![pressure("seat@rig", 99, 100)],
            Coverage {
                evaluated: 1,
                omitted: 0,
                limit_hit: false,
            },
        ),
    };
    let sources: [&dyn HealthSource; 2] = [&broken, &working];
    let projected = run_query(Ok(HealthPolicy::default()), &sources, &query(100)).expect("query");
    assert_eq!(projected.records.len(), 1);
    assert_eq!(projected.records[0].detector, DetectorId::ContextPressure);
    let failed = projected
        .coverage
        .iter()
        .find(|row| row.source == "checkpoints")
        .expect("failure row");
    assert_eq!(failed.status, Some("unavailable"));
    assert!(failed.partial);
    assert_eq!(failed.evaluated, None);
    assert_eq!(failed.omitted, None);
    assert_eq!(failed.reason.as_deref(), Some("broken checkpoint JSON"));
    let json = serde_json::to_value(failed).expect("json");
    assert!(json.get("evaluated").is_none());
    assert!(json.get("limit_hit").is_none());
    assert!(run_query(Ok(HealthPolicy::default()), &sources, &query(201)).is_err());
}

#[test]
fn policy_error_is_not_a_successful_read() {
    let source = Counting {
        name: "context",
        detectors: &[DetectorId::ContextPressure],
        hits: AtomicU32::new(0),
        outcome: Outcome::Ready(
            vec![],
            Coverage {
                evaluated: 0,
                omitted: 0,
                limit_hit: false,
            },
        ),
    };
    let sources: [&dyn HealthSource; 1] = [&source];
    let err = run_query(
        Err(Error::Policy("policy corrupted".into())),
        &sources,
        &query(100),
    )
    .expect_err("policy");
    assert!(err.to_string().contains("policy corrupted"));
    assert_eq!(source.hits.load(Ordering::Relaxed), 0);
}

#[test]
fn disabled_detector_is_not_read() {
    let source = Counting {
        name: "context.pressure",
        detectors: &[DetectorId::ContextPressure],
        hits: AtomicU32::new(0),
        outcome: Outcome::Fail("source offline"),
    };
    let sources: [&dyn HealthSource; 1] = [&source];
    let enabled = run_query(Ok(HealthPolicy::default()), &sources, &query(100)).expect("enabled");
    assert_eq!(enabled.coverage.len(), 1);
    assert_eq!(source.hits.load(Ordering::Relaxed), 1);
    let disabled = HealthPolicy::default().without(&[DetectorId::ContextPressure]);
    let quiet = run_query(Ok(disabled), &sources, &query(100)).expect("disabled");
    assert_eq!(quiet.records, []);
    assert_eq!(quiet.coverage, []);
    assert_eq!(source.hits.load(Ordering::Relaxed), 1);
}

#[test]
fn coverage_limit_reports_omitted_lineages() {
    let notes: Vec<TransitionNote> = (0..100)
        .map(|index| TransitionNote {
            lineage_id: QItemId::new(format!("row-{index}")).expect("id"),
            note: "wake-attempt: 1/3 outcome=failed".to_owned(),
        })
        .collect();
    let (observations, coverage) = wake_from_notes(&notes, 40, at());
    assert_eq!(coverage.evaluated, 40);
    assert_eq!(coverage.omitted, 60);
    assert!(coverage.limit_hit);
    assert_eq!(observations.len(), 40);
    let kept: Vec<&str> = observations
        .iter()
        .map(|observation| match observation {
            HealthObservation::WakeLineage { lineage_id, .. } => lineage_id.as_str(),
            HealthObservation::CoordinationLineage { .. }
            | HealthObservation::Directive { .. }
            | HealthObservation::ScopeAdmission { .. }
            | HealthObservation::ContextPressure { .. } => "",
        })
        .collect();
    assert!(!kept.contains(&"row-99"));
}

#[test]
fn context_thresholds_are_95_and_99() {
    let policy = HealthPolicy::default();
    let quiet = [pressure("a@rig", 50, 100), pressure("b@rig", 85, 100)];
    let reads = [crewmux_health::SourceRead {
        name: "live-context".into(),
        result: Ok((
            quiet.to_vec(),
            Some(Coverage {
                evaluated: 2,
                omitted: 0,
                limit_hit: false,
            }),
        )),
    }];
    let none = project(&policy, &reads, &query(100)).expect("quiet");
    assert_eq!(none.records, []);

    let warning = project(
        &policy,
        &[crewmux_health::SourceRead {
            name: "live-context".into(),
            result: Ok((
                vec![pressure("c@rig", 97, 100)],
                Some(Coverage {
                    evaluated: 1,
                    omitted: 0,
                    limit_hit: false,
                }),
            )),
        }],
        &query(100),
    )
    .expect("warning");
    assert_eq!(warning.records.len(), 1);
    assert_eq!(
        warning.records[0].severity,
        crewmux_health::Severity::Warning
    );
    assert!(warning.records[0].threshold.contains("95%"));
    assert!(warning.records[0].threshold.contains("99%"));

    let critical = project(
        &policy,
        &[crewmux_health::SourceRead {
            name: "live-context".into(),
            result: Ok((
                vec![pressure("d@rig", 99, 100)],
                Some(Coverage {
                    evaluated: 1,
                    omitted: 0,
                    limit_hit: false,
                }),
            )),
        }],
        &query(100),
    )
    .expect("critical");
    assert_eq!(
        critical.records[0].severity,
        crewmux_health::Severity::Critical
    );
    let zero = project(
        &policy,
        &[crewmux_health::SourceRead {
            name: "live-context".into(),
            result: Ok((
                vec![pressure("e@rig", 10, 0)],
                Some(Coverage {
                    evaluated: 1,
                    omitted: 0,
                    limit_hit: false,
                }),
            )),
        }],
        &query(100),
    )
    .expect("zero limit");
    assert_eq!(zero.records, []);
}

#[test]
fn ceremony_is_reserved_ahead_of_context_samples() {
    let mut observations = Vec::new();
    observations.push(HealthObservation::CoordinationLineage {
        lineage_id: "family".into(),
        coordination_transitions: 20,
        product_state_changes: None,
        bounded_authority: false,
        review_returns: 0,
        candidate_changes: 0,
        new_risk_classes: 0,
        at: at(),
    });
    for index in 0..100 {
        observations.push(pressure(&format!("seat-{index}@rig"), 97, 100));
    }
    let projected = project(
        &HealthPolicy::default(),
        &[crewmux_health::SourceRead {
            name: "mixed".into(),
            result: Ok((
                observations,
                Some(Coverage {
                    evaluated: 101,
                    omitted: 0,
                    limit_hit: false,
                }),
            )),
        }],
        &query(100),
    )
    .expect("page");
    assert!(projected.truncated);
    assert_eq!(projected.total, 101);
    assert_eq!(projected.records.len(), 100);
    assert_eq!(
        projected.records[0].detector,
        DetectorId::CeremonyAmplification
    );
    assert_eq!(projected.schema, "openrig.health-list/v0alpha1");
    assert_eq!(projected.records[0].schema, "openrig.health/v0alpha1");
}

#[test]
fn failed_wake_without_escalation_is_a_finding() {
    let notes = vec![TransitionNote {
        lineage_id: QItemId::new("row-a").expect("id"),
        note: "wake-attempt: 1/3 outcome=failed".into(),
    }];
    let (observations, _) = wake_from_notes(&notes, 100, at());
    let projected = project(
        &HealthPolicy::default(),
        &[crewmux_health::SourceRead {
            name: "wake-lineage".into(),
            result: Ok((
                observations,
                Some(Coverage {
                    evaluated: 1,
                    omitted: 0,
                    limit_hit: false,
                }),
            )),
        }],
        &query(100),
    )
    .expect("wake");
    assert_eq!(projected.records.len(), 1);
    assert_eq!(projected.records[0].detector, DetectorId::WakeLineage);
}

#[test]
fn escalated_wake_is_quiet() {
    let notes = vec![
        TransitionNote {
            lineage_id: QItemId::new("row-a").expect("id"),
            note: "wake-attempt: 1/3 outcome=failed".into(),
        },
        TransitionNote {
            lineage_id: QItemId::new("row-a").expect("id"),
            note: "escalation-rung: orchestrator".into(),
        },
    ];
    let (observations, _) = wake_from_notes(&notes, 100, at());
    let projected = project(
        &HealthPolicy::default(),
        &[crewmux_health::SourceRead {
            name: "wake-lineage".into(),
            result: Ok((
                observations,
                Some(Coverage {
                    evaluated: 1,
                    omitted: 0,
                    limit_hit: false,
                }),
            )),
        }],
        &query(100),
    )
    .expect("escalated");
    assert_eq!(projected.records, []);
}

#[test]
fn redundant_wakes_subtract_rescues() {
    let storm = HealthObservation::WakeLineage {
        lineage_id: "row-a".into(),
        failed: false,
        escalated: false,
        wake_count: 4,
        rescue_wake_count: 0,
        existing_next_action: true,
        at: at(),
    };
    let fired = project_one(storm.clone());
    assert_eq!(fired[0].detector, DetectorId::RedundantWakeStorm);
    let mut rescued = storm.clone();
    if let HealthObservation::WakeLineage {
        rescue_wake_count, ..
    } = &mut rescued
    {
        *rescue_wake_count = 1;
    }
    assert_eq!(project_one(rescued), []);
    let mut missing_action = storm;
    if let HealthObservation::WakeLineage {
        existing_next_action,
        ..
    } = &mut missing_action
    {
        *existing_next_action = false;
    }
    assert_eq!(project_one(missing_action), []);
}

#[expect(
    clippy::expect_used,
    reason = "fixture projection; a policy error here is a broken test"
)]
fn project_one(observation: HealthObservation) -> Vec<crewmux_health::HealthRecord> {
    project(
        &HealthPolicy::default(),
        &[crewmux_health::SourceRead {
            name: "wake".into(),
            result: Ok((
                vec![observation],
                Some(Coverage {
                    evaluated: 1,
                    omitted: 0,
                    limit_hit: false,
                }),
            )),
        }],
        &query(100),
    )
    .expect("project")
    .records
}

#[test]
fn node_summary_is_zero_without_nodes() {
    let summary = node_summary(&[]);
    assert_eq!(summary.total, 0);
    assert_eq!(summary.by_session_status, std::collections::BTreeMap::new());
    assert_eq!(summary.by_lifecycle, std::collections::BTreeMap::new());
    assert_eq!(summary.attention_required, 0);
}

#[test]
fn node_summary_counts_session_status() {
    let nodes = [
        NodeFact {
            session_status: String::new(),
            lifecycle: "running".into(),
        },
        NodeFact {
            session_status: String::new(),
            lifecycle: "running".into(),
        },
        NodeFact {
            session_status: "detached".into(),
            lifecycle: "attention_required".into(),
        },
    ];
    let summary = node_summary(&nodes);
    assert_eq!(summary.total, 3);
    assert_eq!(summary.by_session_status.values().sum::<u32>(), 3);
    assert_eq!(summary.by_session_status.get("unknown").copied(), Some(2));
    assert_eq!(summary.attention_required, 1);
}

#[test]
fn context_summary_is_zero_without_samples() {
    let summary = context_summary(&[]);
    assert_eq!(summary.total, 0);
    assert_eq!(summary.critical, 0);
    assert_eq!(summary.warning, 0);
    assert_eq!(summary.stale, 0);
    assert_eq!(summary.by_urgency.get("low").copied(), Some(0));
}

#[test]
fn context_summary_buckets_urgency() {
    let samples = [
        ContextFact {
            used_percentage: Some(92),
            age_seconds: Some(1),
        },
        ContextFact {
            used_percentage: Some(70),
            age_seconds: Some(1),
        },
        ContextFact {
            used_percentage: Some(25),
            age_seconds: Some(1),
        },
        ContextFact {
            used_percentage: Some(0),
            age_seconds: Some(1),
        },
        ContextFact {
            used_percentage: None,
            age_seconds: Some(1),
        },
    ];
    let summary = context_summary(&samples);
    assert_eq!(summary.total, 5);
    assert_eq!(summary.critical, 1);
    assert_eq!(summary.warning, 1);
    assert_eq!(summary.by_urgency.get("low").copied(), Some(2));
    assert_eq!(summary.by_urgency.get("unknown").copied(), Some(1));
}

#[test]
fn context_summary_marks_stale_after_300s() {
    let samples = [
        ContextFact {
            used_percentage: Some(50),
            age_seconds: Some(1),
        },
        ContextFact {
            used_percentage: Some(50),
            age_seconds: Some(600),
        },
        ContextFact {
            used_percentage: Some(50),
            age_seconds: None,
        },
    ];
    let summary = context_summary(&samples);
    assert_eq!(summary.by_freshness.get("fresh").copied(), Some(1));
    assert_eq!(summary.by_freshness.get("stale").copied(), Some(1));
    assert_eq!(summary.by_freshness.get("none").copied(), Some(1));
    assert_eq!(summary.stale, 1);
}

#[test]
fn unknown_limit_is_not_a_zero_percent_sample() {
    let samples = [SeatSample {
        seat: seat("seat@rig"),
        used_tokens: Some(10),
        limit: None,
        at: at(),
    }];
    let (observations, coverage) = context_observations(&samples, 200);
    assert_eq!(observations, []);
    assert_eq!(coverage.evaluated, 1);
    assert!(!coverage.limit_hit);
}

#[test]
fn abstain_requires_a_reason_and_labels_stay_in_the_candidate_set() {
    let candidates = CandidateSet {
        version: "v1".into(),
        values: [(LabelField::ClassificationType, ["note".to_owned()].into())]
            .into_iter()
            .collect(),
        duplicate_candidates: vec![DuplicateCandidate {
            stream_item_id: "other".into(),
            evidence_ref: "evidence-1".into(),
        }],
    };
    assert!(
        validate_decision(
            &candidates,
            &ClassificationDecision::Abstain {
                reason: "   ".into()
            }
        )
        .is_err()
    );
    assert!(
        validate_decision(
            &candidates,
            &ClassificationDecision::Abstain {
                reason: "not enough context".into()
            }
        )
        .is_ok()
    );
    let mut labels = WorkerLabels {
        version: "v1".into(),
        ..WorkerLabels::default()
    };
    labels
        .fields
        .insert(LabelField::ClassificationType, "outside".into());
    assert!(
        validate_decision(
            &candidates,
            &ClassificationDecision::Classify {
                labels: labels.clone()
            }
        )
        .is_err()
    );
    labels
        .fields
        .insert(LabelField::ClassificationType, "note".into());
    labels.duplicate_of = Some("other".into());
    labels.duplicate_evidence = Some("missing".into());
    assert!(
        validate_decision(
            &candidates,
            &ClassificationDecision::Classify {
                labels: labels.clone()
            }
        )
        .is_err()
    );
    labels.duplicate_evidence = Some("evidence-1".into());
    assert!(validate_decision(&candidates, &ClassificationDecision::Classify { labels }).is_ok());
    assert!(cap_page(0).is_err());
    assert!(cap_page(101).is_err());
    assert_eq!(cap_page(100).expect("cap"), 100);
}

#[test]
fn directive_and_admission_tables_report_coverage_none() {
    let source = crewmux_health::AbsentSource {
        name: "directive",
        detectors: &[DetectorId::StaleDirective],
    };
    let sources: [&dyn HealthSource; 1] = [&source];
    let projected = run_query(Ok(HealthPolicy::default()), &sources, &query(100)).expect("none");
    assert_eq!(projected.records, []);
    assert_eq!(projected.coverage[0].status, Some("none"));
    assert!(projected.coverage[0].partial);
    assert_eq!(projected.coverage[0].evaluated, None);
}
