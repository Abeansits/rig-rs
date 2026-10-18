// Derived from openrig packages/daemon/src/domain/watchdog-scheduler.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Durable outbox loop. A due job is claimed once, then pasted through `deliver`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crewmux_orchestration::{Engine, Schedule, is_due, owner_wakes};
use crewmux_store::{NewWatchJob, WatchJob, ensure_job, list_active, mark_terminal};
use tokio::time::interval;

use crate::error::Error;
use crate::health_query::write_db;
use crate::state::App;
use crate::workflow::read_pair;

/// Test knobs. Production leaves the atomics at zero.
#[derive(Clone, Debug)]
pub(crate) struct OutboxCtrl {
    /// Non-zero replaces the wall clock for due checks.
    pub now_ms: Arc<AtomicI64>,
    /// After a claim, return and leave the row `sending`.
    pub pause_after_claim: Arc<AtomicBool>,
    /// After a claim, cancel and do not paste.
    pub cancel_before_paste: Arc<AtomicBool>,
    /// The loop is in `select`. A second start is a no-op until this clears.
    pub running: Arc<AtomicBool>,
    /// Skip ticks. Tests that call [`tick`] set this before [`crate::state::App::launch_workers`].
    pub hold: Arc<AtomicBool>,
}

impl Default for OutboxCtrl {
    fn default() -> Self {
        Self {
            now_ms: Arc::new(AtomicI64::new(0)),
            pause_after_claim: Arc::new(AtomicBool::new(false)),
            cancel_before_paste: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
            hold: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// Run until the daemon's cancellation token fires.
pub(crate) async fn run(app: App) {
    if app.outbox.running.swap(true, Ordering::SeqCst) {
        return;
    }
    let mut ticker = interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            biased;
            () = app.cancel.cancelled() => break,
            _ = ticker.tick() => {
                if app.outbox.hold.load(Ordering::SeqCst) {
                    continue;
                }
                if let Err(err) = tick(&app).await {
                    tracing::warn!(error = %err, "watchdog outbox tick");
                }
            }
        }
    }
    app.outbox.running.store(false, Ordering::SeqCst);
}

/// One pass: reconcile abandoned claims, sync armed jobs, fire what is due.
///
/// # Errors
///
/// Returns [`Error`] when the store cannot be read. One job's failure is logged
/// and does not stop the rest of the pass.
pub(crate) async fn tick(app: &App) -> Result<(), Error> {
    let reconciled = write_db(app, |conn| crewmux_store::reconcile_abandoned(conn)).await?;
    if reconciled > 0 {
        tracing::info!(reconciled, "watchdog claims left sending are indeterminate");
    }
    if app.cancel.is_cancelled() {
        return Ok(());
    }
    let now = now_ms(app);
    let (engine, book) = read_pair(app).await;
    let engine_for_sync = engine.clone();
    let book_for_sync = book.clone();
    write_db(app, move |conn| {
        sync(conn, &engine_for_sync, &book_for_sync, now)
    })
    .await?;
    fire_due(app, &engine, &book, now).await
}

async fn fire_due(
    app: &App,
    engine: &Engine,
    book: &crewmux_messaging::QueueBook,
    now: i64,
) -> Result<(), Error> {
    let jobs = write_db(app, |conn| list_active(conn)).await?;
    for job in jobs {
        if app.cancel.is_cancelled() {
            return Ok(());
        }
        if !is_due(&schedule_of(&job), now) {
            continue;
        }
        if let Err(err) = crate::outbox_fire::fire(app, &job, engine, book, now).await {
            tracing::warn!(job_id = %job.job_id, error = %err, "watchdog job");
        }
    }
    Ok(())
}

pub(crate) fn now_ms(app: &App) -> i64 {
    let over = app.outbox.now_ms.load(Ordering::Relaxed);
    if over != 0 {
        return over;
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

pub(crate) fn schedule_of(job: &WatchJob) -> Schedule<'_> {
    Schedule {
        policy: &job.policy,
        spec_yaml: &job.spec_yaml,
        registered_at_ms: job.registered_at_ms,
        interval_seconds: u64::try_from(job.interval_seconds.max(0)).unwrap_or(0),
        scan_interval_seconds: job
            .scan_interval_seconds
            .and_then(|seconds| u64::try_from(seconds).ok()),
        last_evaluation_at_ms: job.last_evaluation_at_ms,
    }
}

fn sync(
    conn: &crewmux_store::Connection,
    engine: &Engine,
    book: &crewmux_messaging::QueueBook,
    now: i64,
) -> Result<(), crewmux_store::Error> {
    for job in &engine.keepalives {
        let id = format!("keepalive:{}", job.instance_id);
        if !job.armed {
            disarm(conn, &id, job.reason.as_deref().unwrap_or("disarmed"))?;
            continue;
        }
        let interval = i64::try_from(job.interval_seconds).unwrap_or(i64::MAX);
        ensure_job(
            conn,
            &NewWatchJob {
                job_id: id,
                policy: "workflow-keepalive".to_owned(),
                target_session: job.target_session.clone(),
                spec_yaml: job.yaml.clone(),
                message: None,
                interval_seconds: interval.max(1),
                scan_interval_seconds: None,
                active_wake_interval_seconds: None,
                registered_at_ms: now,
                registered_by: "workflow@kernel".to_owned(),
            },
        )?;
    }
    for wake in owner_wakes(engine, book) {
        let target = wake.target.clone();
        ensure_job(
            conn,
            &NewWatchJob {
                job_id: wake.job_id,
                policy: "owner-wake".to_owned(),
                target_session: target.clone(),
                spec_yaml: format!("policy: owner-wake\ntarget:\n  session: {target}\n"),
                message: Some(wake.message),
                interval_seconds: 60,
                scan_interval_seconds: None,
                active_wake_interval_seconds: None,
                registered_at_ms: now,
                registered_by: "workflow@kernel".to_owned(),
            },
        )?;
    }
    Ok(())
}

fn disarm(
    conn: &crewmux_store::Connection,
    job_id: &str,
    reason: &str,
) -> Result<(), crewmux_store::Error> {
    if crewmux_store::get_job(conn, job_id)?.is_some_and(|job| job.state == "active") {
        mark_terminal(conn, job_id, reason)?;
    }
    Ok(())
}
