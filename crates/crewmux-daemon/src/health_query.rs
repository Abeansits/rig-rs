// Derived from openrig packages/daemon/src/domain/health-detectors.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Build a health projection from stored samples and queue notes.

use std::sync::Arc;

use crewmux_core::Timestamp;
use crewmux_health::{
    AbsentSource, ContextFact, ContextSummary, Coverage, DetectorId, HealthObservation,
    HealthProjection, HealthQuery, HealthSource, NodeFact, NodeSummary, SeatSample, TransitionNote,
    context_observations, context_summary, node_summary, run_query, wake_from_notes,
};
use crewmux_messaging::SessionRef;
use crewmux_store::UsageRow;

use crate::error::Error;
use crate::state::{self, App};

const CONTEXT: [DetectorId; 1] = [DetectorId::ContextPressure];
const WAKE: [DetectorId; 2] = [DetectorId::WakeLineage, DetectorId::RedundantWakeStorm];
const DIRECTIVE: [DetectorId; 1] = [DetectorId::StaleDirective];
const ADMISSION: [DetectorId; 1] = [DetectorId::ScopeAdmissionDrift];

/// Project health. A policy error fails the query before any source is read.
///
/// # Errors
///
/// Returns [`Error`] when the database read fails or the policy document is rejected.
pub(crate) async fn projection(
    app: &App,
    limit: u32,
    severity: Option<crewmux_health::Severity>,
    status: Option<crewmux_health::Status>,
) -> Result<HealthProjection, Error> {
    let notes = wake_notes(app).await;
    let now = Timestamp::from_millis(now_i64());
    read_db(app, move |conn| {
        let policy = crewmux_store::policy_load(conn)?;
        let context = ContextSrc { conn };
        let wake = WakeSrc { notes };
        let directive = AbsentSource {
            name: "directive",
            detectors: &DIRECTIVE,
        };
        let admission = AbsentSource {
            name: "scope-admission",
            detectors: &ADMISSION,
        };
        let sources: [&dyn HealthSource; 4] = [&context, &wake, &directive, &admission];
        let query = HealthQuery {
            limit,
            now,
            severity,
            status,
        };
        run_query(Ok(policy), &sources, &query).map_err(crewmux_store::Error::from)
    })
    .await
}

/// Node rollup from seat rows. An empty table is zeros.
///
/// # Errors
///
/// Returns [`Error`] when the seat query fails.
pub(crate) async fn nodes(app: &App) -> Result<NodeSummary, Error> {
    let states = read_db(app, crewmux_store::seat_states).await?;
    let facts: Vec<NodeFact> = states
        .into_iter()
        .map(|state| NodeFact {
            session_status: state.clone(),
            lifecycle: state,
        })
        .collect();
    Ok(node_summary(&facts))
}

/// Context urgency rollup. Unknown percents stay unknown.
///
/// # Errors
///
/// Returns [`Error`] when the usage query fails.
pub(crate) async fn contexts(app: &App) -> Result<ContextSummary, Error> {
    let now = now_i64();
    let rows = read_db(app, crewmux_store::usage_list).await?;
    let facts: Vec<ContextFact> = rows
        .iter()
        .map(|row| ContextFact {
            used_percentage: used_percent(row),
            age_seconds: age_seconds(row.sampled_at_ms, now),
        })
        .collect();
    Ok(context_summary(&facts))
}

pub(crate) async fn read_db<T: Send + 'static>(
    app: &App,
    job: impl FnOnce(&crewmux_store::Connection) -> Result<T, crewmux_store::Error> + Send + 'static,
) -> Result<T, Error> {
    let store = Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || store.read(job))
        .await
        .map_err(state::join_err)?
        .map_err(Error::from)
}

pub(crate) async fn write_db<T: Send + 'static>(
    app: &App,
    job: impl FnOnce(&mut crewmux_store::Connection) -> Result<T, crewmux_store::Error> + Send + 'static,
) -> Result<T, Error> {
    let store = Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || store.write(job))
        .await
        .map_err(state::join_err)?
        .map_err(Error::from)
}

pub(crate) fn now_i64() -> i64 {
    i64::try_from(state::now_ms()).unwrap_or(i64::MAX)
}

async fn wake_notes(app: &App) -> Vec<TransitionNote> {
    let book = app.queue.lock().await;
    let mut notes = Vec::new();
    for item in book.items() {
        for transition in book.transitions(&item.id) {
            notes.push(TransitionNote {
                lineage_id: item.id.clone(),
                note: transition.note,
                // Stand-in until the queue stores a next-action field: a
                // non-empty body counts as an obligation the wake duplicates.
                next_action: !item.body.is_empty(),
            });
        }
    }
    notes
}

struct ContextSrc<'a> {
    conn: &'a crewmux_store::Connection,
}

impl HealthSource for ContextSrc<'_> {
    fn name(&self) -> &'static str {
        "live-context"
    }

    fn detectors(&self) -> &[DetectorId] {
        &CONTEXT
    }

    fn read(
        &self,
        _query: &HealthQuery,
    ) -> Result<(Vec<HealthObservation>, Option<Coverage>), crewmux_health::Error> {
        let rows = crewmux_store::usage_list(self.conn)
            .map_err(|err| crewmux_health::Error::Rejected(err.to_string()))?;
        let mut samples = Vec::new();
        for row in rows {
            if let Ok(sample) = sample(row) {
                samples.push(sample);
            }
        }
        Ok(bounded(context_observations(&samples, 200)))
    }
}

struct WakeSrc {
    notes: Vec<TransitionNote>,
}

impl HealthSource for WakeSrc {
    fn name(&self) -> &'static str {
        "wake-lineage"
    }

    fn detectors(&self) -> &[DetectorId] {
        &WAKE
    }

    fn read(
        &self,
        query: &HealthQuery,
    ) -> Result<(Vec<HealthObservation>, Option<Coverage>), crewmux_health::Error> {
        Ok(bounded(wake_from_notes(
            &self.notes,
            query.limit,
            query.now,
        )))
    }
}

fn bounded(read: (Vec<HealthObservation>, Coverage)) -> (Vec<HealthObservation>, Option<Coverage>) {
    (read.0, Some(read.1))
}

fn sample(row: UsageRow) -> Result<SeatSample, crewmux_health::Error> {
    let seat = SessionRef::new(row.seat)
        .map_err(|err| crewmux_health::Error::Rejected(err.to_string()))?;
    Ok(SeatSample {
        seat,
        used_tokens: widen(row.used_tokens),
        limit: widen(row.token_limit),
        at: Timestamp::from_millis(row.sampled_at_ms),
    })
}

fn widen(value: Option<i64>) -> Option<u64> {
    value.and_then(|number| u64::try_from(number).ok())
}

fn used_percent(row: &UsageRow) -> Option<u32> {
    let used = widen(row.used_tokens)?;
    let limit = widen(row.token_limit)?;
    if limit == 0 {
        return None;
    }
    u32::try_from(used.saturating_mul(100) / limit).ok()
}

fn age_seconds(sampled_at_ms: i64, now_ms: i64) -> Option<u64> {
    let age_ms = now_ms.saturating_sub(sampled_at_ms);
    u64::try_from(age_ms / 1000).ok()
}
