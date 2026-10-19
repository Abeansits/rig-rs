// Derived from openrig packages/daemon/src/domain/watchdog-jobs-repository.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
// Derived from openrig packages/daemon/src/domain/watchdog-history-log.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Durable watchdog jobs, the append-only history, and the exactly-once claim.

use rusqlite::{Connection, OptionalExtension, params};
use ulid::Ulid;

use crate::error::Error;

/// Policies this outbox fires.
pub const POLICIES: [&str; 4] = [
    "workflow-keepalive",
    "workflow-deadline",
    "owner-wake",
    "periodic-reminder",
];

/// One active or finished job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchJob {
    /// Stable id.
    pub job_id: String,
    /// Policy name.
    pub policy: String,
    /// Registered session.
    pub target_session: String,
    /// Spec YAML.
    pub spec_yaml: String,
    /// Authored body, when the policy does not build one.
    pub message: Option<String>,
    /// `active`, `stopped`, or `terminal`.
    pub state: String,
    /// Cadence in seconds.
    pub interval_seconds: i64,
    /// Scan cadence. Absent uses `interval_seconds`.
    pub scan_interval_seconds: Option<i64>,
    /// Wake throttle. Absent means every due scan may send.
    pub active_wake_interval_seconds: Option<i64>,
    /// Registration instant.
    pub registered_at_ms: i64,
    /// Previous evaluation. Absent means never evaluated.
    pub last_evaluation_at_ms: Option<i64>,
    /// Previous paste.
    pub last_fire_at_ms: Option<i64>,
    /// Overdue identity already pasted.
    pub last_fired_condition: Option<String>,
    /// Last evaluation wanted a send.
    pub actionable: bool,
    /// When it first became actionable in this window.
    pub last_actionable_at_ms: Option<i64>,
    /// Why a terminal job stopped.
    pub terminal_reason: Option<String>,
    /// Who registered it.
    pub registered_by: String,
}

/// Fields for a new active job.
pub struct NewWatchJob {
    /// Stable id.
    pub job_id: String,
    /// Policy name.
    pub policy: String,
    /// Registered session. Must contain `@`.
    pub target_session: String,
    /// Spec YAML.
    pub spec_yaml: String,
    /// Authored body.
    pub message: Option<String>,
    /// Positive cadence.
    pub interval_seconds: i64,
    /// Optional scan cadence.
    pub scan_interval_seconds: Option<i64>,
    /// Optional wake throttle.
    pub active_wake_interval_seconds: Option<i64>,
    /// Registration instant.
    pub registered_at_ms: i64,
    /// Registrar.
    pub registered_by: String,
}

/// One history row. The log has no update and no delete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchHistory {
    /// ULID.
    pub id: String,
    /// Job.
    pub job_id: String,
    /// Evaluation instant.
    pub evaluated_at_ms: i64,
    /// `sent`, `skipped`, or `terminal`.
    pub outcome: String,
    /// Skip or terminal reason.
    pub skip_reason: Option<String>,
    /// Paste target.
    pub delivery_target: Option<String>,
    /// `ok` or `failed`.
    pub delivery_status: Option<String>,
    /// Authored body.
    pub delivery_message: Option<String>,
    /// JSON notes.
    pub notes: String,
}

/// Insert input for [`append_history`].
pub struct NewHistory {
    /// Job.
    pub job_id: String,
    /// Evaluation instant.
    pub evaluated_at_ms: i64,
    /// `sent`, `skipped`, or `terminal`.
    pub outcome: String,
    /// Reason, when this was not a send.
    pub skip_reason: Option<String>,
    /// Paste target.
    pub delivery_target: Option<String>,
    /// Delivery status.
    pub delivery_status: Option<String>,
    /// Authored body.
    pub delivery_message: Option<String>,
    /// JSON notes.
    pub notes: String,
}

/// A claim that won the insert, plus the stamps to restore if the paste does not happen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreshClaim {
    /// Occurrence id.
    pub occurrence_id: String,
    /// `last_evaluation_at_ms` before this claim.
    pub prior_evaluation_ms: Option<i64>,
    /// `last_fire_at_ms` before this claim.
    pub prior_fire_ms: Option<i64>,
}

/// Result of trying to own an occurrence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// This caller owns the paste.
    Fresh(FreshClaim),
    /// The occurrence was already claimed.
    Held {
        /// Stored claim status.
        status: String,
    },
}

/// Register `job` or refresh an active row's target and spec.
///
/// A stopped or terminal row is left alone: an operator stop survives a re-arm.
///
/// # Errors
///
/// Returns [`Error::Watchdog`] when the policy, interval, or session is rejected.
pub fn ensure_job(conn: &Connection, job: &NewWatchJob) -> Result<WatchJob, Error> {
    validate(job)?;
    if let Some(existing) = get_job(conn, &job.job_id)? {
        if existing.state == "active" {
            conn.execute(
                "UPDATE watchdog_jobs
                 SET target_session = ?1, spec_yaml = ?2, message = ?3,
                     interval_seconds = ?4, scan_interval_seconds = ?5,
                     active_wake_interval_seconds = ?6
                 WHERE job_id = ?7",
                params![
                    job.target_session,
                    job.spec_yaml,
                    job.message,
                    job.interval_seconds,
                    job.scan_interval_seconds,
                    job.active_wake_interval_seconds,
                    job.job_id,
                ],
            )?;
        }
        return get_job(conn, &job.job_id)?
            .ok_or_else(|| Error::Watchdog("job_not_found".to_owned()));
    }
    conn.execute(
        "INSERT INTO watchdog_jobs (
            job_id, policy, target_session, spec_yaml, message, state,
            interval_seconds, scan_interval_seconds, active_wake_interval_seconds,
            registered_at_ms, actionable, registered_by
         ) VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7, ?8, ?9, 0, ?10)",
        params![
            job.job_id,
            job.policy,
            job.target_session,
            job.spec_yaml,
            job.message,
            job.interval_seconds,
            job.scan_interval_seconds,
            job.active_wake_interval_seconds,
            job.registered_at_ms,
            job.registered_by,
        ],
    )?;
    get_job(conn, &job.job_id)?.ok_or_else(|| Error::Watchdog("job_not_found".to_owned()))
}

/// Active jobs in registration order.
///
/// # Errors
///
/// Returns a SQLite error.
pub fn list_active(conn: &Connection) -> Result<Vec<WatchJob>, Error> {
    let mut stmt = conn.prepare(
        "SELECT job_id, policy, target_session, spec_yaml, message, state,
                interval_seconds, scan_interval_seconds, active_wake_interval_seconds,
                registered_at_ms, last_evaluation_at_ms, last_fire_at_ms, last_fired_condition,
                actionable, last_actionable_at_ms, terminal_reason, registered_by
         FROM watchdog_jobs WHERE state = 'active' ORDER BY rowid ASC",
    )?;
    let rows = stmt.query_map([], row_job)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// One job.
///
/// # Errors
///
/// Returns a SQLite error.
pub fn get_job(conn: &Connection, job_id: &str) -> Result<Option<WatchJob>, Error> {
    conn.query_row(
        "SELECT job_id, policy, target_session, spec_yaml, message, state,
                interval_seconds, scan_interval_seconds, active_wake_interval_seconds,
                registered_at_ms, last_evaluation_at_ms, last_fire_at_ms, last_fired_condition,
                actionable, last_actionable_at_ms, terminal_reason, registered_by
         FROM watchdog_jobs WHERE job_id = ?1",
        params![job_id],
        row_job,
    )
    .optional()
    .map_err(Error::from)
}

/// Stamp the evaluation. `fired` also stamps `last_fire_at_ms`.
///
/// # Errors
///
/// Returns [`Error::Watchdog`] `job_not_found`.
pub fn record_evaluation(
    conn: &Connection,
    job_id: &str,
    at_ms: i64,
    fired: bool,
) -> Result<(), Error> {
    let changed = if fired {
        conn.execute(
            "UPDATE watchdog_jobs
             SET last_evaluation_at_ms = ?1, last_fire_at_ms = ?1
             WHERE job_id = ?2",
            params![at_ms, job_id],
        )?
    } else {
        conn.execute(
            "UPDATE watchdog_jobs SET last_evaluation_at_ms = ?1 WHERE job_id = ?2",
            params![at_ms, job_id],
        )?
    };
    if changed == 0 {
        return Err(Error::Watchdog("job_not_found".to_owned()));
    }
    Ok(())
}

/// Mark the job terminal.
///
/// # Errors
///
/// Returns [`Error::Watchdog`] `job_not_found`.
pub fn mark_terminal(conn: &Connection, job_id: &str, reason: &str) -> Result<(), Error> {
    let changed = conn.execute(
        "UPDATE watchdog_jobs SET state = 'terminal', terminal_reason = ?1 WHERE job_id = ?2",
        params![reason, job_id],
    )?;
    if changed == 0 {
        return Err(Error::Watchdog("job_not_found".to_owned()));
    }
    Ok(())
}

/// Stop an active job. A terminal job is `job_terminal`.
///
/// # Errors
///
/// Returns [`Error::Watchdog`] when the job is missing or already terminal.
pub fn stop_job(conn: &Connection, job_id: &str, reason: &str) -> Result<(), Error> {
    let Some(job) = get_job(conn, job_id)? else {
        return Err(Error::Watchdog("job_not_found".to_owned()));
    };
    if job.state == "terminal" {
        return Err(Error::Watchdog("job_terminal".to_owned()));
    }
    conn.execute(
        "UPDATE watchdog_jobs SET state = 'stopped', terminal_reason = ?1 WHERE job_id = ?2",
        params![reason, job_id],
    )?;
    Ok(())
}

/// Set the actionable flag. `preserve_actionable_at` keeps the existing stamp.
///
/// # Errors
///
/// Returns [`Error::Watchdog`] `job_not_found`.
pub fn set_actionable(
    conn: &Connection,
    job_id: &str,
    actionable: bool,
    at_ms: i64,
    preserve_actionable_at: Option<i64>,
) -> Result<(), Error> {
    let changed = if actionable {
        let stamp = preserve_actionable_at.unwrap_or(at_ms);
        conn.execute(
            "UPDATE watchdog_jobs SET actionable = 1, last_actionable_at_ms = ?1 WHERE job_id = ?2",
            params![stamp, job_id],
        )?
    } else {
        conn.execute(
            "UPDATE watchdog_jobs SET actionable = 0, last_actionable_at_ms = NULL WHERE job_id = ?1",
            params![job_id],
        )?
    };
    if changed == 0 {
        return Err(Error::Watchdog("job_not_found".to_owned()));
    }
    Ok(())
}

/// Remember an overdue identity after a successful paste.
///
/// # Errors
///
/// Returns [`Error::Watchdog`] `job_not_found`.
pub fn record_condition(conn: &Connection, job_id: &str, condition: &str) -> Result<(), Error> {
    let changed = conn.execute(
        "UPDATE watchdog_jobs SET last_fired_condition = ?1 WHERE job_id = ?2",
        params![condition, job_id],
    )?;
    if changed == 0 {
        return Err(Error::Watchdog("job_not_found".to_owned()));
    }
    Ok(())
}

/// Append one history row. There is no update and no delete.
///
/// # Errors
///
/// Returns a SQLite error, including a foreign-key failure for an unknown job.
pub fn append_history(conn: &Connection, row: &NewHistory) -> Result<WatchHistory, Error> {
    let id = Ulid::generate().to_string();
    conn.execute(
        "INSERT INTO watchdog_history (
            id, job_id, evaluated_at_ms, outcome, skip_reason,
            delivery_target, delivery_status, delivery_message, notes
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            id,
            row.job_id,
            row.evaluated_at_ms,
            row.outcome,
            row.skip_reason,
            row.delivery_target,
            row.delivery_status,
            row.delivery_message,
            row.notes,
        ],
    )?;
    Ok(WatchHistory {
        id,
        job_id: row.job_id.clone(),
        evaluated_at_ms: row.evaluated_at_ms,
        outcome: row.outcome.clone(),
        skip_reason: row.skip_reason.clone(),
        delivery_target: row.delivery_target.clone(),
        delivery_status: row.delivery_status.clone(),
        delivery_message: row.delivery_message.clone(),
        notes: row.notes.clone(),
    })
}

/// History for `job_id`, newest evaluation first.
///
/// # Errors
///
/// Returns a SQLite error.
pub fn list_history(conn: &Connection, job_id: &str) -> Result<Vec<WatchHistory>, Error> {
    let mut stmt = conn.prepare(
        "SELECT id, job_id, evaluated_at_ms, outcome, skip_reason,
                delivery_target, delivery_status, delivery_message, notes
         FROM watchdog_history WHERE job_id = ?1
         ORDER BY evaluated_at_ms DESC, id DESC",
    )?;
    let rows = stmt.query_map(params![job_id], row_history)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Claim `occurrence_id` and stamp the evaluation in one transaction.
///
/// A second caller gets [`ClaimOutcome::Held`] and the job stamps stay as they were.
///
/// # Errors
///
/// Returns [`Error::Watchdog`] `job_not_found`.
pub fn claim_occurrence(
    conn: &Connection,
    occurrence_id: &str,
    job_id: &str,
    at_ms: i64,
    target: &str,
    message: &str,
) -> Result<ClaimOutcome, Error> {
    let tx = conn.unchecked_transaction()?;
    let Some(prior) = get_job(&tx, job_id)? else {
        return Err(Error::Watchdog("job_not_found".to_owned()));
    };
    let inserted = tx.execute(
        "INSERT INTO watchdog_claims (
            occurrence_id, job_id, claimed_at_ms, status, target_session, message,
            prior_evaluation_ms, prior_fire_ms
         ) VALUES (?1, ?2, ?3, 'sending', ?4, ?5, ?6, ?7)
         ON CONFLICT(occurrence_id) DO NOTHING",
        params![
            occurrence_id,
            job_id,
            at_ms,
            target,
            message,
            prior.last_evaluation_at_ms,
            prior.last_fire_at_ms,
        ],
    )?;
    if inserted == 0 {
        let status = claim_status(&tx, occurrence_id)?.unwrap_or_else(|| "held".to_owned());
        tx.commit()?;
        return Ok(ClaimOutcome::Held { status });
    }
    record_evaluation(&tx, job_id, at_ms, true)?;
    tx.commit()?;
    Ok(ClaimOutcome::Fresh(FreshClaim {
        occurrence_id: occurrence_id.to_owned(),
        prior_evaluation_ms: prior.last_evaluation_at_ms,
        prior_fire_ms: prior.last_fire_at_ms,
    }))
}

/// Finish a `sending` claim.
///
/// # Errors
///
/// Returns a SQLite error.
pub fn finish_claim(conn: &Connection, occurrence_id: &str, status: &str) -> Result<(), Error> {
    conn.execute(
        "UPDATE watchdog_claims SET status = ?1 WHERE occurrence_id = ?2 AND status = 'sending'",
        params![status, occurrence_id],
    )?;
    Ok(())
}

/// Drop a claim that never pasted and restore the evaluation stamps.
///
/// # Errors
///
/// Returns a SQLite error.
pub fn release_claim(conn: &Connection, claim: &FreshClaim) -> Result<(), Error> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE watchdog_jobs
         SET last_evaluation_at_ms = ?1, last_fire_at_ms = ?2
         WHERE job_id = (SELECT job_id FROM watchdog_claims WHERE occurrence_id = ?3)",
        params![
            claim.prior_evaluation_ms,
            claim.prior_fire_ms,
            claim.occurrence_id,
        ],
    )?;
    tx.execute(
        "DELETE FROM watchdog_claims WHERE occurrence_id = ?1 AND status = 'sending'",
        params![claim.occurrence_id],
    )?;
    tx.commit()?;
    Ok(())
}

/// Claims left `sending` by a crashed process become `indeterminate` and are not resent.
///
/// # Errors
///
/// Returns a SQLite error.
pub fn reconcile_abandoned(conn: &Connection) -> Result<u32, Error> {
    let changed = conn.execute(
        "UPDATE watchdog_claims SET status = 'indeterminate' WHERE status = 'sending'",
        [],
    )?;
    u32::try_from(changed).map_err(|_| Error::Watchdog("claim count overflow".to_owned()))
}

fn validate(job: &NewWatchJob) -> Result<(), Error> {
    if !POLICIES.contains(&job.policy.as_str()) {
        return Err(Error::Watchdog("policy_unknown".to_owned()));
    }
    if job.interval_seconds <= 0 {
        return Err(Error::Watchdog("interval_invalid".to_owned()));
    }
    if !job.target_session.contains('@') {
        return Err(Error::Watchdog("target_session_invalid".to_owned()));
    }
    Ok(())
}

fn claim_status(conn: &Connection, occurrence_id: &str) -> Result<Option<String>, Error> {
    conn.query_row(
        "SELECT status FROM watchdog_claims WHERE occurrence_id = ?1",
        params![occurrence_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(Error::from)
}

fn row_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<WatchJob> {
    Ok(WatchJob {
        job_id: row.get(0)?,
        policy: row.get(1)?,
        target_session: row.get(2)?,
        spec_yaml: row.get(3)?,
        message: row.get(4)?,
        state: row.get(5)?,
        interval_seconds: row.get(6)?,
        scan_interval_seconds: row.get(7)?,
        active_wake_interval_seconds: row.get(8)?,
        registered_at_ms: row.get(9)?,
        last_evaluation_at_ms: row.get(10)?,
        last_fire_at_ms: row.get(11)?,
        last_fired_condition: row.get(12)?,
        actionable: row.get::<_, i64>(13)? != 0,
        last_actionable_at_ms: row.get(14)?,
        terminal_reason: row.get(15)?,
        registered_by: row.get(16)?,
    })
}

fn row_history(row: &rusqlite::Row<'_>) -> rusqlite::Result<WatchHistory> {
    Ok(WatchHistory {
        id: row.get(0)?,
        job_id: row.get(1)?,
        evaluated_at_ms: row.get(2)?,
        outcome: row.get(3)?,
        skip_reason: row.get(4)?,
        delivery_target: row.get(5)?,
        delivery_status: row.get(6)?,
        delivery_message: row.get(7)?,
        notes: row.get(8)?,
    })
}
