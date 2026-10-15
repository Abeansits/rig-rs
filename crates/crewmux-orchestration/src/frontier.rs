// Derived from openrig packages/daemon/src/domain/workflow-frontier-guard.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Out-of-band closes of a live frontier packet are refused.

use crate::error::Error;
use crate::model::Engine;

/// Message and code when `qitem` is a live frontier packet and `next` is a terminal close.
///
/// Claim and `blocked` are not closes. A missing workflow table is an empty engine.
#[must_use]
pub fn frontier_block(engine: &Engine, qitem: &str, next: &str) -> Option<Error> {
    if !matches!(
        next,
        "done" | "handed-off" | "canceled" | "failed" | "denied"
    ) {
        return None;
    }
    let live = engine.instances.iter().find(|instance| {
        instance.status.is_live() && instance.frontier.iter().any(|id| id.as_str() == qitem)
    })?;
    Some(Error::reject(
        "workflow_frontier_packet",
        format!(
            "qitem {qitem} is the LIVE frontier packet of workflow instance {} ({}). Closing it out-of-band would strand the workflow. Use the workflow verbs instead: rig workflow project (advance) | rig workflow route (re-target the owner).",
            live.id, live.workflow_name
        ),
        400,
    ))
}
