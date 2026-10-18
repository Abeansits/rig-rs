//! Jobs and history from the TypeScript watchdog repositories this loop uses.
#![expect(
    clippy::expect_used,
    reason = "setup failures abort the integration test"
)]

use std::path::Path;

use crewmux_store::{
    Error, NewHistory, NewWatchJob, Store, append_history, ensure_job, get_job, list_active,
    list_history, mark_terminal, record_evaluation, set_actionable, stop_job,
};

fn open() -> Store {
    Store::open(Path::new(":memory:")).expect("db")
}

fn job(id: &str, policy: &str, target: &str, interval: i64) -> NewWatchJob {
    NewWatchJob {
        job_id: id.to_owned(),
        policy: policy.to_owned(),
        target_session: target.to_owned(),
        spec_yaml: format!("policy: {policy}\ntarget:\n  session: {target}\nmessage: hello\n"),
        message: None,
        interval_seconds: interval,
        scan_interval_seconds: None,
        active_wake_interval_seconds: None,
        registered_at_ms: 1_000,
        registered_by: "ops@kernel".to_owned(),
    }
}

fn expect_code(err: &Error, code: &str) {
    let rendered = err.to_string();
    assert!(rendered.contains(code), "{rendered}");
}

#[test]
fn register_accepts_workflow_keepalive() {
    let store = open();
    let saved = store
        .write(|conn| ensure_job(conn, &job("k", "workflow-keepalive", "a@rig", 60)))
        .expect("register");
    assert_eq!(saved.policy, "workflow-keepalive");
    assert_eq!(saved.state, "active");
    assert!(!saved.actionable);
}

#[test]
fn register_rejects_unknown_policy() {
    let store = open();
    let err = store
        .write(|conn| ensure_job(conn, &job("k", "totally-bogus", "a@rig", 60)))
        .expect_err("unknown");
    expect_code(&err, "policy_unknown");
}

#[test]
fn register_rejects_non_positive_interval() {
    let store = open();
    for interval in [0, -1] {
        let err = store
            .write(move |conn| ensure_job(conn, &job("k", "periodic-reminder", "a@rig", interval)))
            .expect_err("interval");
        expect_code(&err, "interval_invalid");
    }
}

#[test]
fn register_rejects_target_session_without_at() {
    let store = open();
    let err = store
        .write(|conn| ensure_job(conn, &job("k", "periodic-reminder", "no-at-here", 60)))
        .expect_err("target");
    expect_code(&err, "target_session_invalid");
}

#[test]
fn list_active_returns_only_active_jobs_in_registration_order() {
    let store = open();
    store
        .write(|conn| {
            ensure_job(conn, &job("a", "periodic-reminder", "a@rig", 60))?;
            ensure_job(conn, &job("b", "periodic-reminder", "b@rig", 60))?;
            ensure_job(conn, &job("c", "periodic-reminder", "c@rig", 60))?;
            stop_job(conn, "b", "operator")?;
            let ids: Vec<String> = list_active(conn)?
                .into_iter()
                .map(|row| row.job_id)
                .collect();
            assert_eq!(ids, vec!["a".to_owned(), "c".to_owned()]);
            Ok(())
        })
        .expect("list");
}

#[test]
fn record_evaluation_fired_true_updates_both_stamps() {
    let store = open();
    store
        .write(|conn| {
            ensure_job(conn, &job("a", "periodic-reminder", "a@rig", 60))?;
            record_evaluation(conn, "a", 5_000, true)?;
            let row = get_job(conn, "a")?.expect("row");
            assert_eq!(row.last_evaluation_at_ms, Some(5_000));
            assert_eq!(row.last_fire_at_ms, Some(5_000));
            Ok(())
        })
        .expect("fired");
}

#[test]
fn record_evaluation_fired_false_updates_only_evaluation() {
    let store = open();
    store
        .write(|conn| {
            ensure_job(conn, &job("a", "periodic-reminder", "a@rig", 60))?;
            record_evaluation(conn, "a", 5_000, true)?;
            record_evaluation(conn, "a", 8_000, false)?;
            let row = get_job(conn, "a")?.expect("row");
            assert_eq!(row.last_evaluation_at_ms, Some(8_000));
            assert_eq!(row.last_fire_at_ms, Some(5_000));
            Ok(())
        })
        .expect("quiet");
}

#[test]
fn mark_terminal_sets_state_and_reason() {
    let store = open();
    store
        .write(|conn| {
            ensure_job(conn, &job("a", "periodic-reminder", "a@rig", 60))?;
            mark_terminal(conn, "a", "done")?;
            let row = get_job(conn, "a")?.expect("row");
            assert_eq!(row.state, "terminal");
            assert_eq!(row.terminal_reason.as_deref(), Some("done"));
            Ok(())
        })
        .expect("terminal");
}

#[test]
fn stop_sets_stopped_and_records_reason() {
    let store = open();
    store
        .write(|conn| {
            ensure_job(conn, &job("a", "periodic-reminder", "a@rig", 60))?;
            stop_job(conn, "a", "operator")?;
            let row = get_job(conn, "a")?.expect("row");
            assert_eq!(row.state, "stopped");
            assert_eq!(row.terminal_reason.as_deref(), Some("operator"));
            Ok(())
        })
        .expect("stop");
}

#[test]
fn stop_on_terminal_job_throws_job_terminal() {
    let store = open();
    let err = store
        .write(|conn| {
            ensure_job(conn, &job("a", "periodic-reminder", "a@rig", 60))?;
            mark_terminal(conn, "a", "done")?;
            stop_job(conn, "a", "again")
        })
        .expect_err("terminal");
    expect_code(&err, "job_terminal");
}

#[test]
fn set_actionable_true_stamps_evaluated_at() {
    let store = open();
    store
        .write(|conn| {
            ensure_job(conn, &job("a", "periodic-reminder", "a@rig", 60))?;
            set_actionable(conn, "a", true, 9_000, None)?;
            let row = get_job(conn, "a")?.expect("row");
            assert!(row.actionable);
            assert_eq!(row.last_actionable_at_ms, Some(9_000));
            Ok(())
        })
        .expect("actionable");
}

#[test]
fn set_actionable_true_preserves_last_actionable_at() {
    let store = open();
    store
        .write(|conn| {
            ensure_job(conn, &job("a", "periodic-reminder", "a@rig", 60))?;
            set_actionable(conn, "a", true, 9_000, None)?;
            set_actionable(conn, "a", true, 12_000, Some(9_000))?;
            let row = get_job(conn, "a")?.expect("row");
            assert_eq!(row.last_actionable_at_ms, Some(9_000));
            Ok(())
        })
        .expect("preserve");
}

#[test]
fn set_actionable_false_clears() {
    let store = open();
    store
        .write(|conn| {
            ensure_job(conn, &job("a", "periodic-reminder", "a@rig", 60))?;
            set_actionable(conn, "a", true, 9_000, None)?;
            set_actionable(conn, "a", false, 12_000, None)?;
            let row = get_job(conn, "a")?.expect("row");
            assert!(!row.actionable);
            assert_eq!(row.last_actionable_at_ms, None);
            Ok(())
        })
        .expect("clear");
}

#[test]
fn get_missing_job_is_job_not_found() {
    let store = open();
    store
        .write(|conn| {
            assert!(get_job(conn, "missing")?.is_none());
            Ok(())
        })
        .expect("get");
    let err = store
        .write(|conn| record_evaluation(conn, "missing", 1, false))
        .expect_err("missing");
    expect_code(&err, "job_not_found");
}

#[test]
fn record_sent_persists_delivery_target_status_message_and_ulid_id() {
    let store = open();
    let row = store
        .write(|conn| {
            ensure_job(conn, &job("a", "periodic-reminder", "a@rig", 60))?;
            append_history(conn, &sent("a", 1_000, "hello"))
        })
        .expect("sent");
    assert_eq!(row.id.len(), 26);
    assert!(
        row.id
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
    );
    assert_eq!(row.outcome, "sent");
    assert_eq!(row.delivery_target.as_deref(), Some("a@rig"));
    assert_eq!(row.delivery_status.as_deref(), Some("ok"));
    assert_eq!(row.delivery_message.as_deref(), Some("hello"));
}

#[test]
fn record_skipped_persists_reason_and_null_delivery_fields() {
    let store = open();
    let row = store
        .write(|conn| {
            ensure_job(conn, &job("a", "periodic-reminder", "a@rig", 60))?;
            append_history(
                conn,
                &NewHistory {
                    job_id: "a".to_owned(),
                    evaluated_at_ms: 1_000,
                    outcome: "skipped".to_owned(),
                    skip_reason: Some("no_actionable_artifacts".to_owned()),
                    delivery_target: None,
                    delivery_status: None,
                    delivery_message: None,
                    notes: "{}".to_owned(),
                },
            )
        })
        .expect("skipped");
    assert_eq!(row.outcome, "skipped");
    assert_eq!(row.skip_reason.as_deref(), Some("no_actionable_artifacts"));
    assert_eq!(row.delivery_target, None);
}

#[test]
fn record_terminal_persists_reason() {
    let store = open();
    let row = store
        .write(|conn| {
            ensure_job(conn, &job("a", "periodic-reminder", "a@rig", 60))?;
            append_history(
                conn,
                &NewHistory {
                    job_id: "a".to_owned(),
                    evaluated_at_ms: 1_000,
                    outcome: "terminal".to_owned(),
                    skip_reason: Some("policy_done".to_owned()),
                    delivery_target: None,
                    delivery_status: None,
                    delivery_message: None,
                    notes: "{}".to_owned(),
                },
            )
        })
        .expect("terminal");
    assert_eq!(row.outcome, "terminal");
    assert_eq!(row.skip_reason.as_deref(), Some("policy_done"));
}

#[test]
fn record_json_encodes_evaluation_notes_and_decodes_on_read() {
    let store = open();
    let notes = serde_json::json!({"artifact_count": 7, "label": "foo"}).to_string();
    store
        .write(move |conn| {
            ensure_job(conn, &job("a", "periodic-reminder", "a@rig", 60))?;
            let mut row = sent("a", 1_000, "x");
            row.notes = notes;
            append_history(conn, &row)?;
            let listed = list_history(conn, "a")?;
            let parsed: serde_json::Value =
                serde_json::from_str(&listed[0].notes).expect("notes json");
            assert_eq!(parsed["artifact_count"], 7);
            assert_eq!(parsed["label"], "foo");
            Ok(())
        })
        .expect("notes");
}

#[test]
fn list_for_job_returns_desc_by_evaluated_at() {
    let store = open();
    store
        .write(|conn| {
            ensure_job(conn, &job("a", "periodic-reminder", "a@rig", 60))?;
            append_history(conn, &sent("a", 1_000, "first"))?;
            append_history(conn, &sent("a", 2_000, "second"))?;
            let listed = list_history(conn, "a")?;
            assert_eq!(listed[0].delivery_message.as_deref(), Some("second"));
            assert_eq!(listed[1].delivery_message.as_deref(), Some("first"));
            Ok(())
        })
        .expect("order");
}

#[test]
fn count_for_job_returns_the_number_of_recorded_entries() {
    let store = open();
    store
        .write(|conn| {
            ensure_job(conn, &job("a", "periodic-reminder", "a@rig", 60))?;
            assert_eq!(list_history(conn, "a")?.len(), 0);
            append_history(conn, &sent("a", 1_000, "one"))?;
            append_history(conn, &sent("a", 1_000, "two"))?;
            assert_eq!(list_history(conn, "a")?.len(), 2);
            Ok(())
        })
        .expect("count");
}

#[test]
fn fk_violation_record_against_unknown_job_id_throws() {
    let store = open();
    let err = store
        .write(|conn| append_history(conn, &sent("unknown-job-id", 1_000, "x")))
        .expect_err("fk");
    let rendered = err.to_string();
    assert!(
        rendered.to_ascii_lowercase().contains("foreign key"),
        "{rendered}"
    );
}

#[test]
fn api_surface_does_not_expose_update_or_delete() {
    let source = include_str!("../src/watchdog.rs");
    for name in [
        "update_history",
        "delete_history",
        "remove_history",
        "modify_history",
    ] {
        assert!(!source.contains(&format!("fn {name}")), "{name}");
    }
    let store = open();
    store
        .write(|conn| {
            ensure_job(conn, &job("a", "periodic-reminder", "a@rig", 60))?;
            let first = append_history(conn, &sent("a", 1_000, "first"))?;
            append_history(conn, &sent("a", 2_000, "second"))?;
            let listed = list_history(conn, "a")?;
            assert_eq!(listed[1].id, first.id);
            assert_eq!(listed[1].delivery_message.as_deref(), Some("first"));
            Ok(())
        })
        .expect("append only");
}

#[test]
fn claim_is_unique_release_restores_and_abandoned_sending_is_indeterminate() {
    let store = open();
    store
        .write(|conn| {
            ensure_job(conn, &job("a", "periodic-reminder", "a@rig", 60))?;
            record_evaluation(conn, "a", 1_000, false)?;
            let first = crewmux_store::claim_occurrence(conn, "occ-1", "a", 9_000, "a@rig", "hi")?;
            let crewmux_store::ClaimOutcome::Fresh(claim) = first else {
                panic!("fresh");
            };
            let held = crewmux_store::claim_occurrence(conn, "occ-1", "a", 9_000, "a@rig", "hi")?;
            assert!(matches!(held, crewmux_store::ClaimOutcome::Held { .. }));
            crewmux_store::release_claim(conn, &claim)?;
            let row = get_job(conn, "a")?.expect("row");
            assert_eq!(row.last_evaluation_at_ms, Some(1_000));
            assert_eq!(row.last_fire_at_ms, None);
            crewmux_store::claim_occurrence(conn, "occ-2", "a", 9_000, "a@rig", "hi")?;
            assert_eq!(crewmux_store::reconcile_abandoned(conn)?, 1);
            let status: String = conn.query_row(
                "SELECT status FROM watchdog_claims WHERE occurrence_id = 'occ-2'",
                [],
                |row| row.get(0),
            )?;
            assert_eq!(status, "indeterminate");
            Ok(())
        })
        .expect("claim");
}

fn sent(job_id: &str, at: i64, message: &str) -> NewHistory {
    NewHistory {
        job_id: job_id.to_owned(),
        evaluated_at_ms: at,
        outcome: "sent".to_owned(),
        skip_reason: None,
        delivery_target: Some("a@rig".to_owned()),
        delivery_status: Some("ok".to_owned()),
        delivery_message: Some(message.to_owned()),
        notes: "{}".to_owned(),
    }
}
