// Derived from openrig packages/daemon/src/domain/watchdog-policy-engine.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! One job: decide, claim, paste through [`crate::deliver::deliver`].

use std::sync::atomic::Ordering;

use crewmux_core::Timestamp;
use crewmux_messaging::QueueBook;
use crewmux_orchestration::{
    Engine, KeepaliveJob, WatchAction, WatchFail, evaluate_keepalive, is_quiet, next_due_at,
    occurrence_id, parse_armed,
};
use crewmux_store::{
    ClaimOutcome, FreshClaim, NewHistory, WatchJob, append_history, claim_occurrence, finish_claim,
    record_condition, record_evaluation, release_claim, set_actionable,
};
use crewmux_types::QItemId;
use serde_json::{Value, json};

use crate::deliver::{Outbound, deliver};
use crate::error::Error;
use crate::health_query::write_db;
use crate::outbox::schedule_of;
use crate::state::App;

pub(crate) async fn fire(
    app: &App,
    job: &WatchJob,
    engine: &Engine,
    book: &QueueBook,
    now: i64,
) -> Result<(), Error> {
    if throttled(job, now) {
        return stamp_throttle(app, job, now).await;
    }
    let action = decide(job, engine, book, now)?;
    match action {
        WatchAction::Skip { reason } => skip(app, job, now, reason).await,
        WatchAction::Terminal { reason, notes } => terminal(app, job, now, &reason, &notes).await,
        WatchAction::Send {
            target,
            message,
            condition,
            notes,
        } => {
            send(
                app,
                job,
                now,
                &target,
                &message,
                condition.as_deref(),
                &notes,
            )
            .await
        }
    }
}

fn decide(
    job: &WatchJob,
    engine: &Engine,
    book: &QueueBook,
    now: i64,
) -> Result<WatchAction, Error> {
    if job.policy == "owner-wake" {
        let message = job.message.clone().unwrap_or_default();
        if message.is_empty() {
            return Ok(WatchAction::Terminal {
                reason: "owner_wake_empty".to_owned(),
                notes: json!({ "jobId": job.job_id }),
            });
        }
        return Ok(WatchAction::Send {
            target: job.target_session.clone(),
            message,
            condition: Some(job.job_id.clone()),
            notes: json!({ "packet": job.job_id }),
        });
    }
    if job.policy == "periodic-reminder" {
        return reminder(job);
    }
    if job.policy != "workflow-keepalive" && job.policy != "workflow-deadline" {
        return Ok(WatchAction::Terminal {
            reason: format!("unknown_policy:{}", job.policy),
            notes: json!({ "policy": job.policy }),
        });
    }
    let spec = parse_armed(&job.spec_yaml);
    let observers = Vec::new();
    let evaluation = evaluate_keepalive(
        engine,
        book,
        &KeepaliveJob {
            job_id: &job.job_id,
            target_session: &job.target_session,
            message: job.message.as_deref(),
            instance_id: Some(spec.instance_id.as_str()).filter(|id| !id.is_empty()),
            packet_id: None,
            observer_session: Some(spec.observer_session.as_str()).filter(|id| !id.is_empty()),
            observer_sessions: &observers,
            deadline_gated: spec.deadline_gated,
            last_fired_condition: job.last_fired_condition.as_deref(),
        },
        Timestamp::from_millis(now),
    );
    evaluation.map_err(|WatchFail| Error::Message("policy_spec_invalid".to_owned()))
}

fn reminder(job: &WatchJob) -> Result<WatchAction, Error> {
    if job.target_session.is_empty() {
        return Err(Error::Message("policy_spec_invalid".to_owned()));
    }
    let spec = parse_armed(&job.spec_yaml);
    let from_column = job.message.as_deref().filter(|text| !text.is_empty());
    let from_spec = (!spec.message.is_empty()).then_some(spec.message.as_str());
    let Some(message) = from_column.or(from_spec) else {
        return Err(Error::Message("policy_spec_invalid".to_owned()));
    };
    Ok(WatchAction::Send {
        target: job.target_session.clone(),
        message: message.to_owned(),
        condition: None,
        notes: json!({}),
    })
}

fn throttled(job: &WatchJob, now: i64) -> bool {
    let (Some(window), Some(last_fire)) = (job.active_wake_interval_seconds, job.last_fire_at_ms)
    else {
        return false;
    };
    if !job.actionable || window <= 0 {
        return false;
    }
    now.saturating_sub(last_fire) < window.saturating_mul(1_000)
}

async fn stamp_throttle(app: &App, job: &WatchJob, now: i64) -> Result<(), Error> {
    let job_id = job.job_id.clone();
    let preserve = job.last_actionable_at_ms;
    write_db(app, move |conn| {
        record_evaluation(conn, &job_id, now, false)?;
        set_actionable(conn, &job_id, true, now, preserve)?;
        Ok(())
    })
    .await
}

async fn skip(app: &App, job: &WatchJob, now: i64, reason: &'static str) -> Result<(), Error> {
    let job_id = job.job_id.clone();
    write_db(app, move |conn| {
        record_evaluation(conn, &job_id, now, false)?;
        set_actionable(conn, &job_id, false, now, None)?;
        if !is_quiet(reason) {
            append_history(
                conn,
                &NewHistory {
                    job_id: job_id.clone(),
                    evaluated_at_ms: now,
                    outcome: "skipped".to_owned(),
                    skip_reason: Some(reason.to_owned()),
                    delivery_target: None,
                    delivery_status: None,
                    delivery_message: None,
                    notes: "{}".to_owned(),
                },
            )?;
        }
        Ok(())
    })
    .await
}

async fn terminal(
    app: &App,
    job: &WatchJob,
    now: i64,
    reason: &str,
    notes: &Value,
) -> Result<(), Error> {
    let job_id = job.job_id.clone();
    let reason = reason.to_owned();
    let notes = notes.to_string();
    write_db(app, move |conn| {
        crewmux_store::mark_terminal(conn, &job_id, &reason)?;
        append_history(
            conn,
            &NewHistory {
                job_id,
                evaluated_at_ms: now,
                outcome: "terminal".to_owned(),
                skip_reason: Some(reason),
                delivery_target: None,
                delivery_status: None,
                delivery_message: None,
                notes,
            },
        )?;
        Ok(())
    })
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "one send carries the job, the clock, and the decision together"
)]
async fn send(
    app: &App,
    job: &WatchJob,
    now: i64,
    target: &str,
    message: &str,
    condition: Option<&str>,
    notes: &Value,
) -> Result<(), Error> {
    let receipt = condition.map_or_else(|| due_receipt(job), str::to_owned);
    let claimed = claim_send(app, job, now, target, message, &receipt).await?;
    let ClaimOutcome::Fresh(claim) = claimed else {
        return held(app, job).await;
    };
    if app.outbox.pause_after_claim.swap(false, Ordering::SeqCst) {
        return Ok(());
    }
    if app.outbox.cancel_before_paste.swap(false, Ordering::SeqCst) || app.cancel.is_cancelled() {
        return finish(app, &claim, "cancelled").await;
    }
    after_paste(app, job, now, target, message, condition, notes, &claim).await
}

#[expect(
    clippy::too_many_arguments,
    reason = "the paste is recorded against the claim and the decision"
)]
async fn after_paste(
    app: &App,
    job: &WatchJob,
    now: i64,
    target: &str,
    message: &str,
    condition: Option<&str>,
    notes: &Value,
    claim: &FreshClaim,
) -> Result<(), Error> {
    match paste(app, target, message).await {
        Err(err) => {
            tracing::info!(job_id = %job.job_id, error = %err, "watchdog paste did not start");
            release(app, claim).await
        }
        Ok(value) => {
            record_delivery(
                app, job, now, target, message, condition, notes, claim, &value,
            )
            .await
        }
    }
}

async fn paste(app: &App, target: &str, message: &str) -> Result<Value, Error> {
    deliver(
        app,
        Outbound {
            from: "watchdog@kernel".to_owned(),
            targets: vec![target.to_owned()],
            rig: None,
            all: false,
            body: message.to_owned(),
            interact: false,
            reason: None,
            verify: true,
            track_direct: false,
        },
    )
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "the paste result is recorded against the claim, the job, and the decision"
)]
async fn record_delivery(
    app: &App,
    job: &WatchJob,
    now: i64,
    target: &str,
    message: &str,
    condition: Option<&str>,
    notes: &Value,
    claim: &FreshClaim,
    value: &Value,
) -> Result<(), Error> {
    let outcome = receipt_outcome(value);
    if matches!(outcome, "deferred" | "escalated" | "") {
        return release(app, claim).await;
    }
    let status = if outcome == "delivered" {
        "ok"
    } else {
        "failed"
    };
    let claim_status = if status == "ok" {
        "delivered"
    } else {
        "failed"
    };
    finish(app, claim, claim_status).await?;
    if status == "ok" {
        record_ok(app, job, now, message, condition).await?;
    }
    if job.policy == "owner-wake" {
        close_owner(app, &job.job_id, status).await?;
    }
    remember_actionable(app, job, now).await?;
    write_sent(app, sent_row(job, now, target, message, status, notes)).await
}

async fn record_ok(
    app: &App,
    job: &WatchJob,
    now: i64,
    message: &str,
    condition: Option<&str>,
) -> Result<(), Error> {
    if let Some(condition) = condition {
        remember_condition(app, &job.job_id, condition).await?;
    }
    if job.policy == "owner-wake" {
        mark_delivered_nudge(app, job, message, now).await?;
    }
    Ok(())
}

fn sent_row(
    job: &WatchJob,
    now: i64,
    target: &str,
    message: &str,
    status: &str,
    notes: &Value,
) -> NewHistory {
    NewHistory {
        job_id: job.job_id.clone(),
        evaluated_at_ms: now,
        outcome: "sent".to_owned(),
        skip_reason: None,
        delivery_target: Some(target.to_owned()),
        delivery_status: Some(status.to_owned()),
        delivery_message: Some(message.to_owned()),
        notes: notes.to_string(),
    }
}

async fn claim_send(
    app: &App,
    job: &WatchJob,
    now: i64,
    target: &str,
    message: &str,
    receipt: &str,
) -> Result<ClaimOutcome, Error> {
    let occurrence = occurrence_id(&job.job_id, None, receipt);
    let job_id = job.job_id.clone();
    let target_owned = target.to_owned();
    let message_owned = message.to_owned();
    write_db(app, move |conn| {
        claim_occurrence(
            conn,
            &occurrence,
            &job_id,
            now,
            &target_owned,
            &message_owned,
        )
    })
    .await
}

fn due_receipt(job: &WatchJob) -> String {
    match next_due_at(&schedule_of(job)) {
        None => format!("immediate:{}", job.registered_at_ms),
        Some(due) => format!("due:{due}"),
    }
}

async fn held(app: &App, job: &WatchJob) -> Result<(), Error> {
    if job.policy != "owner-wake" {
        return Ok(());
    }
    let job_id = job.job_id.clone();
    write_db(app, move |conn| {
        if crewmux_store::get_job(conn, &job_id)?.is_some_and(|row| row.state == "active") {
            crewmux_store::mark_terminal(conn, &job_id, "owner_wake_claimed")?;
        }
        Ok(())
    })
    .await
}

async fn finish(app: &App, claim: &FreshClaim, status: &str) -> Result<(), Error> {
    let occurrence = claim.occurrence_id.clone();
    let status = status.to_owned();
    write_db(app, move |conn| finish_claim(conn, &occurrence, &status)).await
}

async fn release(app: &App, claim: &FreshClaim) -> Result<(), Error> {
    let claim = claim.clone();
    write_db(app, move |conn| release_claim(conn, &claim)).await
}

async fn remember_condition(app: &App, job_id: &str, condition: &str) -> Result<(), Error> {
    let job_id = job_id.to_owned();
    let condition = condition.to_owned();
    write_db(app, move |conn| record_condition(conn, &job_id, &condition)).await
}

async fn close_owner(app: &App, job_id: &str, status: &str) -> Result<(), Error> {
    let job_id = job_id.to_owned();
    let reason = if status == "ok" {
        "owner_wake_delivered"
    } else {
        "owner_wake_failed"
    };
    write_db(app, move |conn| {
        crewmux_store::mark_terminal(conn, &job_id, reason)
    })
    .await
}

async fn remember_actionable(app: &App, job: &WatchJob, now: i64) -> Result<(), Error> {
    let job_id = job.job_id.clone();
    let preserve = job.last_actionable_at_ms;
    write_db(app, move |conn| {
        set_actionable(conn, &job_id, true, now, preserve)
    })
    .await
}

async fn write_sent(app: &App, row: NewHistory) -> Result<(), Error> {
    write_db(app, move |conn| append_history(conn, &row).map(|_| ())).await
}

async fn mark_delivered_nudge(
    app: &App,
    job: &WatchJob,
    message: &str,
    now: i64,
) -> Result<(), Error> {
    let Some(packet) = job.job_id.strip_prefix("owner-wake:") else {
        return Ok(());
    };
    let Ok(id) = QItemId::new(packet) else {
        return Ok(());
    };
    let stamped = {
        let mut book = app.queue.lock().await;
        book.append_note(
            &id,
            "workflow.nudge",
            Some(message),
            Some("delivered"),
            Timestamp::from_millis(now),
        )
        .map_err(|err| Error::Message(err.to_string()))?;
        book.clone()
    };
    app.commit_queue(stamped).await
}

fn receipt_outcome(value: &Value) -> &str {
    value
        .get("receipts")
        .and_then(Value::as_array)
        .and_then(|rows| rows.first())
        .and_then(|row| row.get("outcome"))
        .and_then(Value::as_str)
        .unwrap_or("")
}
