//! Tool argument structs. Field names match the renamed schemas.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

/// No arguments.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub(crate) struct NoArgs {}

/// `rig_start`. `sourceRef` is required. The other fields are accepted and not applied.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct RigStartArgs {
    /// Path to a rig spec YAML file.
    #[serde(rename = "sourceRef")]
    pub source_ref: String,
    /// Accepted for the renamed schema. The daemon bring-up has no plan flag.
    pub plan: Option<bool>,
    /// Accepted for the renamed schema. The daemon bring-up has no auto-approve flag.
    #[serde(rename = "autoApprove")]
    pub auto_approve: Option<bool>,
    /// Accepted for the renamed schema. The daemon bring-up has no target root.
    #[serde(rename = "targetRoot")]
    pub target_root: Option<String>,
}

/// `rig_stop`. Optional flags are accepted and not sent.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct RigStopArgs {
    /// Rig id or name the daemon `down` route understands.
    #[serde(rename = "rigId")]
    pub rig_id: String,
    /// Accepted and not applied.
    pub delete: Option<bool>,
    /// Accepted and not applied. This is not `dangerously_interact`.
    pub force: Option<bool>,
    /// Accepted and not applied.
    pub snapshot: Option<bool>,
}

/// `agent_read_screen`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ReadScreenArgs {
    /// Seat or session name.
    pub session: String,
    /// Capture line budget. Omitted uses the daemon default.
    pub lines: Option<u32>,
}

/// `agent_send`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct AgentSendArgs {
    /// Target session name.
    pub session: String,
    /// Message text. The daemon runs lineage and pre-send checks.
    pub text: String,
    /// Ask the daemon to verify delivery.
    pub verify: Option<bool>,
    /// Accepted only so `force` plus `waitForIdleSeconds` can be rejected locally.
    pub force: Option<bool>,
    /// Not implemented. Combined with `force` it is rejected before any HTTP call.
    #[serde(rename = "waitForIdleSeconds")]
    pub wait_for_idle_seconds: Option<f64>,
}

/// One pod-local edge on `agent_add`.
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
pub(crate) struct EdgeArg {
    /// Source member id.
    pub from: String,
    /// Destination member id.
    pub to: String,
    /// Edge kind.
    pub kind: String,
}

/// `agent_add`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct AgentAddArgs {
    /// Rig id, percent-encoded into the path.
    #[serde(rename = "rigId")]
    pub rig_id: String,
    /// Pod namespace, percent-encoded into the path.
    #[serde(rename = "podNamespace")]
    pub pod_namespace: String,
    /// Member fragment. The daemon route is not implemented; the tool still posts it.
    pub member: Value,
    /// Optional pod-local edges.
    pub edges: Option<Vec<EdgeArg>>,
    /// Optional root for agent resolution.
    #[serde(rename = "rigRoot")]
    pub rig_root: Option<String>,
}

/// `chat_send`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ChatSendArgs {
    /// Rig name resolved through `GET /api/rigs/summary`.
    #[serde(rename = "rigName")]
    pub rig_name: String,
    /// Message body.
    pub body: String,
}

/// `chat_read`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ChatReadArgs {
    /// Rig name resolved through `GET /api/rigs/summary`.
    #[serde(rename = "rigName")]
    pub rig_name: String,
}

/// Snapshot create or list.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SnapshotRigArgs {
    /// Rig id sent as `{"rig": ...}`.
    #[serde(rename = "rigId")]
    pub rig_id: String,
}

/// `snapshot_restore`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SnapshotRestoreArgs {
    /// Rig id sent as `{"rig": ...}`.
    #[serde(rename = "rigId")]
    pub rig_id: String,
    /// Snapshot id sent as `{"snapshot": ...}`.
    #[serde(rename = "snapshotId")]
    pub snapshot_id: String,
}

/// `spec_validate_rig` and `spec_validate_agent`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct YamlArgs {
    /// YAML text.
    pub yaml: String,
}
