// Derived from openrig packages/daemon/src/domain/workflow-keepalive-arming.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! One deadline-gated job per instance. Arm and disarm share the projection.

use crate::model::{Engine, Keepalive};

/// Serial keepalive interval: 15 minutes.
pub(crate) const INTERVAL_SECONDS: u64 = 15 * 60;

/// Arm or refresh the instance job. Returns whether this call created it.
pub(crate) fn arm(engine: &mut Engine, instance_id: &str, target: &str) -> bool {
    if let Some(job) = engine
        .keepalives
        .iter_mut()
        .find(|job| job.instance_id == instance_id)
    {
        target.clone_into(&mut job.target_session);
        job.armed = true;
        job.reason = None;
        job.yaml = spec_yaml(instance_id, target);
        return false;
    }
    engine.keepalives.push(Keepalive {
        instance_id: instance_id.to_owned(),
        target_session: target.to_owned(),
        interval_seconds: INTERVAL_SECONDS,
        deadline_gated: true,
        armed: true,
        reason: None,
        yaml: spec_yaml(instance_id, target),
    });
    true
}

/// Disarm every job for the instance.
pub(crate) fn disarm(engine: &mut Engine, instance_id: &str, reason: &str) {
    for job in engine
        .keepalives
        .iter_mut()
        .filter(|job| job.instance_id == instance_id)
    {
        job.armed = false;
        job.reason = Some(reason.to_owned());
    }
}

/// YAML the watchdog would load.
#[must_use]
pub(crate) fn spec_yaml(instance_id: &str, session: &str) -> String {
    format!(
        "policy: workflow-keepalive\ntarget:\n  session: {session}\ncontext:\n  workflow_instance_id: {instance_id}\ndeadline_gated: true\ninterval_seconds: {INTERVAL_SECONDS}\n"
    )
}
