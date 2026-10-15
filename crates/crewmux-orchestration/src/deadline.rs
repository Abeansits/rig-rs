// Derived from openrig packages/daemon/src/domain/workflow-deadline.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Stuck is derived. Nothing stores it.

use crewmux_core::Timestamp;
use crewmux_messaging::{QueueItem, QueueState};

use crate::model::{Instance, Status};

/// Four hours. The one workflow stuck threshold.
pub const STUCK_THRESHOLD_SECONDS: i64 = 4 * 60 * 60;

/// v1 hop baseline.
pub const MAX_HOPS_BASELINE_V1: u32 = 0;

/// Deadline verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    /// `healthy`, `overdue-claimed`, or `overdue-unclaimed`.
    pub state: &'static str,
    /// Evidence. Absent when healthy.
    pub evidence: Option<Evidence>,
}

/// The classification tuple list/show/trace expose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evidence {
    /// Instance.
    pub instance_id: String,
    /// Step.
    pub step_id: Option<String>,
    /// Packet.
    pub packet_id: String,
    /// Owner.
    pub owner_session: String,
    /// Packet state token.
    pub packet_state: String,
    /// Anchor name.
    pub anchor: &'static str,
    /// Anchor instant.
    pub anchor_at: Timestamp,
    /// Seconds past the deadline.
    pub overdue_by_seconds: i64,
    /// Seconds since the packet was created.
    pub age_seconds: i64,
    /// Claim instant, when claimed.
    pub claimed_at: Option<Timestamp>,
}

/// Classify `instance` from its frontier packets.
#[must_use]
pub fn evaluate_deadline(instance: &Instance, packets: &[QueueItem], now: Timestamp) -> Verdict {
    if !matches!(instance.status, Status::Active | Status::Waiting) {
        return healthy();
    }
    let threshold_ms = STUCK_THRESHOLD_SECONDS.saturating_mul(1000);
    for packet in packets {
        if !instance.frontier.iter().any(|id| id == &packet.id) {
            continue;
        }
        if packet.state == QueueState::InProgress {
            if let Some(verdict) = claimed(instance, packet, now, threshold_ms) {
                return verdict;
            }
        } else if packet.state == QueueState::Pending {
            let deadline = packet.created_at.millis().saturating_add(threshold_ms);
            if now.millis() >= deadline {
                return overdue(
                    instance,
                    packet,
                    now,
                    "overdue-unclaimed",
                    "created_at",
                    packet.created_at,
                    deadline,
                );
            }
        }
    }
    healthy()
}

/// True when one more hop would pass an enforceable guard.
#[must_use]
pub fn exceeds_max_hops(hop_count: u32, baseline: u32, max_hops: Option<i64>) -> bool {
    let Some(max_hops) = max_hops else {
        return false;
    };
    if max_hops < 1 {
        return false;
    }
    let Some(max_hops) = u32::try_from(max_hops).ok() else {
        return false;
    };
    hop_count.saturating_add(1).saturating_sub(baseline) > max_hops
}

fn claimed(
    instance: &Instance,
    packet: &QueueItem,
    now: Timestamp,
    threshold_ms: i64,
) -> Option<Verdict> {
    let (anchor, anchor_at, deadline) = if let Some(due) = packet.closure_required_at {
        ("closure_required_at", due, due.millis())
    } else if let Some(claimed_at) = packet.claimed_at {
        (
            "claimed_at",
            claimed_at,
            claimed_at.millis().saturating_add(threshold_ms),
        )
    } else {
        (
            "created_at",
            packet.created_at,
            packet.created_at.millis().saturating_add(threshold_ms),
        )
    };
    if now.millis() < deadline {
        return None;
    }
    Some(overdue(
        instance,
        packet,
        now,
        "overdue-claimed",
        anchor,
        anchor_at,
        deadline,
    ))
}

#[expect(
    clippy::too_many_arguments,
    reason = "the verdict names the anchor, the deadline, and the packet together"
)]
fn overdue(
    instance: &Instance,
    packet: &QueueItem,
    now: Timestamp,
    state: &'static str,
    anchor: &'static str,
    anchor_at: Timestamp,
    deadline_ms: i64,
) -> Verdict {
    let overdue_by = now.millis().saturating_sub(deadline_ms) / 1000;
    let age = now.millis().saturating_sub(packet.created_at.millis()) / 1000;
    Verdict {
        state,
        evidence: Some(Evidence {
            instance_id: instance.id.as_str().to_owned(),
            step_id: instance.current_step_id.clone(),
            packet_id: packet.id.as_str().to_owned(),
            owner_session: packet.destination.as_str().to_owned(),
            packet_state: packet.state.as_str().to_owned(),
            anchor,
            anchor_at,
            overdue_by_seconds: overdue_by.max(0),
            age_seconds: age.max(0),
            claimed_at: packet.claimed_at,
        }),
    }
}

fn healthy() -> Verdict {
    Verdict {
        state: "healthy",
        evidence: None,
    }
}
