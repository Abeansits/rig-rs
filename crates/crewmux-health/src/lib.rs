//! Health detectors, stream classifier, and context-usage monitor.
//!
//! The projection is pure. SQLite lives in `crewmux-store`. The daemon is the
//! only composition root, and it never calls a model to classify a stream item.

// Derived modules name their OpenRig source in the file header.

mod classify;
mod coverage;
mod detector;
mod error;
mod evaluate;
mod lease;
mod lineage;
mod observation;
mod policy;
mod project;
mod record;
mod samples;
mod stream_item;
mod summary;
mod time;

pub use classify::{
    CandidateSet, ClassificationDecision, DuplicateCandidate, LabelField, MAX_ELIGIBLE_PAGE,
    WorkerLabels, cap_page, validate_decision,
};
pub use coverage::{Coverage, CoverageRow};
pub use detector::DetectorId;
pub use error::Error;
pub use lease::{AbsentSource, ClassifierLease, DEFAULT_LEASE_TTL_MS, LeaseState};
pub use lineage::{TransitionNote, wake_from_notes};
pub use observation::HealthObservation;
pub use policy::{HealthPolicy, Thresholds, parse_policy};
pub use project::{
    HealthProjection, HealthQuery, HealthSource, SourceRead, checked_limit, project, run_query,
};
pub use record::{
    HEALTH_LIST_SCHEMA, HEALTH_RECORD_SCHEMA, HealthRecord, Severity, Status, finding_id,
};
pub use samples::{SeatSample, context_observations};
pub use stream_item::StreamItem;
pub use summary::{
    ContextFact, ContextSummary, NodeFact, NodeSummary, context_summary, node_summary,
};
pub use time::{iso_millis, iso_timestamp};
