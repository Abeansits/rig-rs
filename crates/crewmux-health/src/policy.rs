// Derived from openrig packages/daemon/src/domain/health-policy.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Enablement and thresholds. A policy that fails to parse fails the query.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::detector::DetectorId;
use crate::error::Error;

/// Counts and percents the detectors compare against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thresholds {
    /// Coordination transitions before ceremony is a candidate.
    pub ceremony_transitions: u32,
    /// Coordination / product-state ratio.
    pub ceremony_ratio: u32,
    /// Review returns before a carousel finding.
    pub review_returns: u32,
    /// Redundant wakes before a storm finding.
    pub redundant_wakes: u32,
    /// Context percent that opens a warning.
    pub context_warning_percent: u32,
    /// Context percent that opens a critical finding.
    pub context_critical_percent: u32,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            ceremony_transitions: 20,
            ceremony_ratio: 12,
            review_returns: 4,
            redundant_wakes: 4,
            context_warning_percent: 95,
            context_critical_percent: 99,
        }
    }
}

/// Which detectors run, and at which thresholds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HealthPolicy {
    /// Detectors that may produce findings. A detector outside this set is skipped.
    pub enabled: BTreeSet<DetectorId>,
    /// Numeric gates.
    pub thresholds: Thresholds,
}

impl Default for HealthPolicy {
    fn default() -> Self {
        Self {
            enabled: DetectorId::ALL.into_iter().collect(),
            thresholds: Thresholds::default(),
        }
    }
}

impl HealthPolicy {
    /// Whether `id` may be read and may produce a finding.
    #[must_use]
    pub fn allows(&self, id: DetectorId) -> bool {
        self.enabled.contains(&id)
    }

    /// Drop `disabled` from the default policy.
    #[must_use]
    pub fn without(mut self, disabled: &[DetectorId]) -> Self {
        for id in disabled {
            self.enabled.remove(id);
        }
        self
    }
}

/// Parse a stored policy document.
///
/// # Errors
///
/// Returns [`Error::Policy`] when the document is not the v0alpha1 object.
pub fn parse_policy(value: &Value) -> Result<HealthPolicy, Error> {
    let object = value
        .as_object()
        .ok_or_else(|| Error::Policy("expected an object".to_owned()))?;
    let schema = object
        .get("schema")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Policy("missing schema".to_owned()))?;
    if schema != "openrig.health-policy/v0alpha1" {
        return Err(Error::Policy("unsupported health policy schema".to_owned()));
    }
    let mut policy = HealthPolicy::default();
    let disabled = object
        .get("disabledDetectors")
        .ok_or_else(|| Error::Policy("missing disabledDetectors".to_owned()))?;
    policy.enabled = enabled_from(disabled)?;
    if let Some(thresholds) = object.get("thresholds") {
        apply_thresholds(&mut policy.thresholds, thresholds)?;
    }
    if let Some(pressure) = object.get("contextPressure") {
        apply_pressure(&mut policy.thresholds, pressure)?;
    }
    Ok(policy)
}

fn enabled_from(value: &Value) -> Result<BTreeSet<DetectorId>, Error> {
    let list = value
        .as_array()
        .ok_or_else(|| Error::Policy("disabledDetectors must be an array".to_owned()))?;
    let mut enabled: BTreeSet<DetectorId> = DetectorId::ALL.into_iter().collect();
    let mut seen = BTreeSet::new();
    for item in list {
        let name = item
            .as_str()
            .ok_or_else(|| Error::Policy("detector id must be a string".to_owned()))?;
        let id = DetectorId::ALL
            .into_iter()
            .find(|id| id.as_str() == name)
            .ok_or_else(|| Error::Policy(format!("unknown detector {name}")))?;
        if !seen.insert(id) {
            return Err(Error::Policy(format!("duplicate detector {name}")));
        }
        enabled.remove(&id);
    }
    Ok(enabled)
}

fn apply_thresholds(thresholds: &mut Thresholds, value: &Value) -> Result<(), Error> {
    let object = value
        .as_object()
        .ok_or_else(|| Error::Policy("thresholds must be an object".to_owned()))?;
    thresholds.ceremony_transitions = required_u32(object.get("ceremonyTransitions"))?;
    thresholds.ceremony_ratio = required_u32(object.get("ceremonyRatio"))?;
    thresholds.review_returns = required_u32(object.get("reviewReturns"))?;
    thresholds.redundant_wakes = required_u32(object.get("redundantWakes"))?;
    Ok(())
}

fn apply_pressure(thresholds: &mut Thresholds, value: &Value) -> Result<(), Error> {
    let object = value
        .as_object()
        .ok_or_else(|| Error::Policy("contextPressure must be an object".to_owned()))?;
    thresholds.context_warning_percent = required_u32(object.get("warningPercent"))?;
    thresholds.context_critical_percent = required_u32(object.get("criticalPercent"))?;
    Ok(())
}

fn required_u32(value: Option<&Value>) -> Result<u32, Error> {
    let Some(value) = value else {
        return Err(Error::Policy("missing threshold".to_owned()));
    };
    let number = value
        .as_u64()
        .ok_or_else(|| Error::Policy("threshold must be an integer".to_owned()))?;
    u32::try_from(number).map_err(|_| Error::Policy("threshold is out of range".to_owned()))
}
