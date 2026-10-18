// Derived from openrig packages/daemon/src/domain/watchdog-due.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Next evaluation instant. `None` means due now.

/// Fields the scheduler reads. The policy body is not one of them.
pub struct Schedule<'a> {
    /// Policy name. Only `periodic-reminder` waits out its first interval.
    pub policy: &'a str,
    /// Spec text. A JSON spec can carry the pre-upgrade pending-wake flag.
    pub spec_yaml: &'a str,
    /// Registration instant, epoch milliseconds.
    pub registered_at_ms: i64,
    /// Cadence when `scan_interval_seconds` is absent.
    pub interval_seconds: u64,
    /// Scan cadence. Wins over `interval_seconds` when set.
    pub scan_interval_seconds: Option<u64>,
    /// Previous evaluation. Absent means never evaluated. `Some(0)` is due now.
    pub last_evaluation_at_ms: Option<i64>,
}

/// When the scheduler next evaluates `job`, or `None` when it is due now.
#[must_use]
pub fn next_due_at(job: &Schedule<'_>) -> Option<i64> {
    let cadence_ms = i64::try_from(
        job.scan_interval_seconds
            .unwrap_or(job.interval_seconds)
            .saturating_mul(1_000),
    )
    .unwrap_or(i64::MAX);
    match job.last_evaluation_at_ms {
        None => first_due(job, cadence_ms),
        Some(0) => None,
        Some(last) => Some(last.saturating_add(cadence_ms)),
    }
}

/// True when `now_ms` is at or past [`next_due_at`].
#[must_use]
pub fn is_due(job: &Schedule<'_>, now_ms: i64) -> bool {
    next_due_at(job).is_none_or(|due| now_ms >= due)
}

fn first_due(job: &Schedule<'_>, cadence_ms: i64) -> Option<i64> {
    if job.policy != "periodic-reminder" {
        return None;
    }
    if event_pending(job.spec_yaml) {
        return None;
    }
    Some(job.registered_at_ms.saturating_add(cadence_ms))
}

/// TypeScript `JSON.parse(specYaml).context.queue_wait.eventPending`.
///
/// YAML that is not JSON is not the upgrade marker.
fn event_pending(spec_yaml: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(spec_yaml) else {
        return false;
    };
    value
        .get("context")
        .and_then(|context| context.get("queue_wait"))
        .and_then(|wait| wait.get("eventPending"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}
