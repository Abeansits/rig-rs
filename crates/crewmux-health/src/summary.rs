// Derived from openrig packages/daemon/src/domain/steering/health-summary.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Compact node and context rollups. These are not detector findings.

use std::collections::BTreeMap;

use serde::Serialize;

/// One seat for the node rollup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeFact {
    /// Session status token. Empty becomes `unknown`.
    pub session_status: String,
    /// Lifecycle token. `attention_required` increments the headline count.
    pub lifecycle: String,
}

/// Cross-rig node counts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeSummary {
    /// Nodes considered.
    pub total: u32,
    /// Counts by session status.
    pub by_session_status: BTreeMap<String, u32>,
    /// Counts by lifecycle.
    pub by_lifecycle: BTreeMap<String, u32>,
    /// Nodes whose lifecycle is `attention_required`.
    pub attention_required: u32,
}

/// One context sample for the urgency rollup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextFact {
    /// Used percent. `None` is the unknown bucket, not zero.
    pub used_percentage: Option<u32>,
    /// Age of the sample in seconds. `None` is freshness `none`.
    pub age_seconds: Option<u64>,
}

/// Cross-rig context urgency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSummary {
    /// Samples considered.
    pub total: u32,
    /// `critical`, `warning`, `low`, `unknown`.
    pub by_urgency: BTreeMap<String, u32>,
    /// `fresh`, `stale`, `none`.
    pub by_freshness: BTreeMap<String, u32>,
    /// Samples at or above 80%.
    pub critical: u32,
    /// Samples at or above 60% and below 80%.
    pub warning: u32,
    /// Samples at least 600s old.
    pub stale: u32,
}

/// A context sample younger than this is fresh. The summary and the finding agree.
pub(crate) const FRESHNESS_SECONDS: u64 = 600;
const CRITICAL_PERCENT: u32 = 80;
const WARNING_PERCENT: u32 = 60;

/// Roll up node facts. An empty slice is zeros, not an error.
#[must_use]
pub fn node_summary(nodes: &[NodeFact]) -> NodeSummary {
    let mut by_session_status = BTreeMap::new();
    let mut by_lifecycle = BTreeMap::new();
    let mut attention_required: u32 = 0;
    for node in nodes {
        let session = blank_as_unknown(&node.session_status);
        let lifecycle = blank_as_unknown(&node.lifecycle);
        bump(&mut by_session_status, session);
        bump(&mut by_lifecycle, lifecycle);
        if lifecycle == "attention_required" {
            attention_required = attention_required.saturating_add(1);
        }
    }
    NodeSummary {
        total: u32::try_from(nodes.len()).unwrap_or(u32::MAX),
        by_session_status,
        by_lifecycle,
        attention_required,
    }
}

/// Roll up context samples. Urgency is 80 / 60. Freshness is 600s.
#[must_use]
pub fn context_summary(samples: &[ContextFact]) -> ContextSummary {
    let mut by_urgency = zeros(&["critical", "warning", "low", "unknown"]);
    let mut by_freshness = zeros(&["fresh", "stale", "none"]);
    let mut critical: u32 = 0;
    let mut warning: u32 = 0;
    let mut stale: u32 = 0;
    for sample in samples {
        match sample.used_percentage {
            None => bump(&mut by_urgency, "unknown"),
            Some(pct) if pct >= CRITICAL_PERCENT => {
                bump(&mut by_urgency, "critical");
                critical = critical.saturating_add(1);
            }
            Some(pct) if pct >= WARNING_PERCENT => {
                bump(&mut by_urgency, "warning");
                warning = warning.saturating_add(1);
            }
            Some(_) => bump(&mut by_urgency, "low"),
        }
        match sample.age_seconds {
            None => bump(&mut by_freshness, "none"),
            Some(age) if age >= FRESHNESS_SECONDS => {
                bump(&mut by_freshness, "stale");
                stale = stale.saturating_add(1);
            }
            Some(_) => bump(&mut by_freshness, "fresh"),
        }
    }
    ContextSummary {
        total: u32::try_from(samples.len()).unwrap_or(u32::MAX),
        by_urgency,
        by_freshness,
        critical,
        warning,
        stale,
    }
}

fn zeros(keys: &[&str]) -> BTreeMap<String, u32> {
    keys.iter().map(|key| ((*key).to_owned(), 0)).collect()
}

fn bump(map: &mut BTreeMap<String, u32>, key: &str) {
    let entry = map.entry(key.to_owned()).or_insert(0);
    *entry = entry.saturating_add(1);
}

fn blank_as_unknown(value: &str) -> &str {
    if value.is_empty() { "unknown" } else { value }
}
