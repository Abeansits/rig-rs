// Derived from openrig packages/daemon/src/domain/workflow-types.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! In-memory workflow document. One JSON blob in the store.

use std::collections::BTreeMap;

use crewmux_core::Timestamp;
use crewmux_messaging::SessionRef;
use crewmux_types::{QItemId, WorkflowInstanceId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Crash checkpoint inside one projection. The published pair is unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(
    clippy::enum_variant_names,
    reason = "the After prefix is the crash-hook name shared with the store"
)]
pub enum Phase {
    /// Frontier packet closed, successor not created.
    AfterClose,
    /// Successor created, frontier not moved.
    AfterCreate,
    /// Frontier moved, trail not appended.
    AfterFrontier,
    /// Trail appended, keepalive not armed or disarmed.
    AfterTrail,
    /// Keepalive updated, commit not returned.
    AfterKeepalive,
}

impl Phase {
    /// Stable hook name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AfterClose => "after-close",
            Self::AfterCreate => "after-create",
            Self::AfterFrontier => "after-frontier",
            Self::AfterTrail => "after-trail",
            Self::AfterKeepalive => "after-keepalive",
        }
    }
}

/// Instance status. The wire tokens match TypeScript.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Accepts `project`.
    Active,
    /// Parked. Still accepts `project`.
    Waiting,
    /// Terminal success.
    Completed,
    /// Terminal failure.
    Failed,
    /// Operator abort.
    Aborted,
}

impl Status {
    /// Wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Waiting => "waiting",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Aborted => "aborted",
        }
    }

    /// `active` or `waiting`.
    #[must_use]
    pub const fn is_live(self) -> bool {
        matches!(self, Self::Active | Self::Waiting)
    }
}

/// Cached spec bytes. The engine reparses them; it does not keep a second shape.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedSpec {
    /// ULID.
    pub spec_id: String,
    /// Spec id (`linear-build`).
    pub name: String,
    /// `String(version)`.
    pub version: String,
    /// Raw YAML.
    pub yaml: String,
    /// Path the operator passed.
    pub source_path: String,
    /// SHA-256 of `yaml`.
    pub source_hash: String,
    /// When the cache row was written.
    pub cached_at: Timestamp,
    /// Built-in starter shipped with the binary.
    pub built_in: bool,
}

/// Last committed continuation. Waiting replay compares this.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    /// Recorded exit.
    pub exit: String,
    /// Actor session.
    pub actor_session: String,
    /// Packet that was closed.
    pub closed_packet: String,
    /// Successor, when one was minted.
    pub next_packet: Option<String>,
    /// Result note. Absent is null on the wire.
    pub result_note: Option<String>,
    /// Blocker. Absent is null on the wire.
    pub blocked_on: Option<String>,
    /// Step that closed.
    pub current_step: String,
    /// Branch target, when the exit was mapped.
    pub branch_taken: Option<String>,
}

/// One workflow instance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instance {
    /// ULID.
    pub id: WorkflowInstanceId,
    /// Spec name.
    pub workflow_name: String,
    /// Spec version label.
    pub workflow_version: String,
    /// Creator session.
    pub created_by: SessionRef,
    /// Created.
    pub created_at: Timestamp,
    /// Status.
    pub status: Status,
    /// Live frontier packet ids.
    pub frontier: Vec<QItemId>,
    /// Step the frontier represents.
    pub current_step_id: Option<String>,
    /// Hops taken.
    pub hop_count: u32,
    /// Reserved. Always absent.
    pub fallback_synthesis: Option<String>,
    /// Last committed decision.
    pub last_decision: Option<Decision>,
    /// Terminal instant.
    pub completed_at: Option<Timestamp>,
    /// Optimistic version. Instantiate returns 1.
    pub version: u32,
    /// Resume count.
    pub resume_count: u32,
    /// Hop baseline for `max_hops`.
    pub hops_baseline: u32,
    /// Bound rig name.
    pub bound_rig: Option<String>,
}

/// One trail row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trail {
    /// ULID.
    pub id: String,
    /// Instance.
    pub instance_id: String,
    /// Step.
    pub step_id: String,
    /// Role.
    pub step_role: String,
    /// When.
    pub closed_at: Timestamp,
    /// Workflow exit, not the queue closure reason.
    pub closure_reason: String,
    /// Evidence object, or null.
    pub closure_evidence: Option<Value>,
    /// Actor.
    pub actor_session: String,
    /// Successor packet.
    pub next_qitem_id: Option<String>,
    /// Closed packet.
    pub prior_qitem_id: String,
}

/// One keepalive job. Terminal exits disarm it in the same projection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keepalive {
    /// Instance.
    pub instance_id: String,
    /// Owner session the job addresses.
    pub target_session: String,
    /// Interval. Serial jobs use 15 minutes.
    pub interval_seconds: u64,
    /// Deadline-gated.
    pub deadline_gated: bool,
    /// Armed.
    pub armed: bool,
    /// Disarm reason, when disarmed.
    pub reason: Option<String>,
    /// Spec YAML the watchdog would read.
    pub yaml: String,
}

/// The workflow document.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Engine {
    /// Specs keyed by `name@version`.
    #[serde(default)]
    pub specs: BTreeMap<String, CachedSpec>,
    /// Instances in insertion order.
    #[serde(default)]
    pub instances: Vec<Instance>,
    /// Trail rows in append order.
    #[serde(default)]
    pub trails: Vec<Trail>,
    /// One keepalive per instance.
    #[serde(default)]
    pub keepalives: Vec<Keepalive>,
}

impl Engine {
    /// Borrow an instance.
    #[must_use]
    pub fn instance(&self, id: &str) -> Option<&Instance> {
        self.instances.iter().find(|row| row.id.as_str() == id)
    }

    /// Mutably borrow an instance.
    pub fn instance_mut(&mut self, id: &str) -> Option<&mut Instance> {
        self.instances.iter_mut().find(|row| row.id.as_str() == id)
    }

    /// Cache key.
    #[must_use]
    pub fn spec_key(name: &str, version: &str) -> String {
        format!("{name}@{version}")
    }
}
