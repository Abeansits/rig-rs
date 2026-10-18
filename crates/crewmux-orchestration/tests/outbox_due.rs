//! Due math from `watchdog-scheduler.test.ts` and `watchdog-due.ts`.

use crewmux_orchestration::{Schedule, is_due};

fn schedule<'a>(
    policy: &'a str,
    spec: &'a str,
    registered_at_ms: i64,
    interval_seconds: u64,
    scan_interval_seconds: Option<u64>,
    last_evaluation_at_ms: Option<i64>,
) -> Schedule<'a> {
    Schedule {
        policy,
        spec_yaml: spec,
        registered_at_ms,
        interval_seconds,
        scan_interval_seconds,
        last_evaluation_at_ms,
    }
}

#[test]
fn is_due_waits_a_full_interval_for_a_never_evaluated_periodic_reminder() {
    let spec = "context:\n  target:\n    session: a@rig\n  message: x\n";
    let job = schedule("periodic-reminder", spec, 1_000, 30, None, None);
    assert!(!is_due(&job, 1_000));
    assert!(!is_due(&job, 1_000 + 29_999));
    assert!(is_due(&job, 1_000 + 30_000));
}

#[test]
fn is_due_is_true_immediately_for_a_never_evaluated_non_periodic_job() {
    let job = schedule("artifact-pool-ready", "context:\n", 1_000, 30, None, None);
    assert!(is_due(&job, 9_000));
}

#[test]
fn pre_upgrade_pending_wake_is_immediately_due() {
    let pending =
        r#"{"policy":"periodic-reminder","context":{"queue_wait":{"eventPending":true}}}"#;
    let pending = schedule("periodic-reminder", pending, 1_000, 300, None, None);
    assert!(is_due(&pending, 1_000));
    let fresh = r#"{"policy":"periodic-reminder","context":{"queue_wait":{"eventPending":false}}}"#;
    let fresh = schedule("periodic-reminder", fresh, 1_000, 300, None, None);
    assert!(!is_due(&fresh, 1_000));
}

#[test]
fn is_due_returns_false_when_interval_not_yet_elapsed() {
    let job = schedule(
        "periodic-reminder",
        "message: x\n",
        0,
        30,
        None,
        Some(10_000),
    );
    assert!(!is_due(&job, 20_000));
}

#[test]
fn is_due_returns_true_when_interval_elapsed() {
    let job = schedule(
        "periodic-reminder",
        "message: x\n",
        0,
        30,
        None,
        Some(1_000),
    );
    assert!(is_due(&job, 1_000 + 60_000));
}

#[test]
fn is_due_prefers_scan_interval_30_over_interval_600() {
    let job = schedule(
        "periodic-reminder",
        "message: x\n",
        0,
        600,
        Some(30),
        Some(1_000),
    );
    assert!(is_due(&job, 1_000 + 45_000));
}

#[test]
fn is_due_prefers_scan_interval_600_over_interval_30() {
    let job = schedule(
        "periodic-reminder",
        "message: x\n",
        0,
        30,
        Some(600),
        Some(1_000),
    );
    assert!(!is_due(&job, 1_000 + 45_000));
}

#[test]
fn a_forward_jump_is_one_due_instant_not_one_per_missed_interval() {
    let job = schedule("workflow-keepalive", "", 0, 30, None, Some(1_000));
    assert!(is_due(&job, 1_000 + 30_000 * 10));
    let stamped = schedule(
        "workflow-keepalive",
        "",
        0,
        30,
        None,
        Some(1_000 + 30_000 * 10),
    );
    assert!(!is_due(&stamped, 1_000 + 30_000 * 10));
}

#[test]
fn a_backward_clock_before_the_cadence_is_not_due() {
    let job = schedule("workflow-keepalive", "", 0, 30, None, Some(50_000));
    assert!(!is_due(&job, 40_000));
}
