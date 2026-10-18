//! `rig compact`: prep, then `/compact`, only through submit when the seat is `Ready`.

use std::path::PathBuf;

use crewmux_core::Seat;
use crewmux_health::compaction::{
    AttemptId, AttemptState, EnforcerInput, EnforcerOutcome, EnforcerSkipReason, MANUAL_PREP_WAIT,
    ManualCompactionStage, PrepMode, PrepStatus, PreparationState, Threshold, gated_submit,
};
use crewmux_types::{RuntimeKind, SeatId};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::Error;
use crate::query::find_seat;
use crate::state::{App, now_ms};
use crate::submit::submit_text;

use super::persist::save;
use super::{CompactPlan, Runtime};

#[derive(Deserialize)]
struct TriggerBody {
    seat: String,
    #[serde(default)]
    skip_map: bool,
    #[serde(default)]
    instruction: String,
    used_percent: Option<u8>,
    threshold: Option<Value>,
}

#[derive(Deserialize)]
struct SeatBody {
    seat: String,
}

pub(crate) async fn trigger(app: &App, body: &[u8]) -> Result<Value, Error> {
    let parsed: TriggerBody =
        serde_json::from_slice(body).map_err(|err| Error::Message(err.to_string()))?;
    let seat = find_seat(app, &parsed.seat).await?;
    if seat.runtime != RuntimeKind::ClaudeCode {
        return Err(Error::Message("runtime_filter".to_owned()));
    }
    let ready = super::apply::seat_is_ready(app, &seat).await;
    drive(app, &seat, &parsed, ready).await
}

pub(crate) async fn cancel(app: &App, body: &[u8]) -> Result<Value, Error> {
    let parsed: SeatBody =
        serde_json::from_slice(body).map_err(|err| Error::Message(err.to_string()))?;
    let seat = find_seat(app, &parsed.seat).await?;
    let key = seat.id.to_string();
    let session = seat.name.as_str().to_owned();
    {
        let mut guard = app.compaction.lock().await;
        guard.book.cancel(&key);
        guard.enforcer.cancel(&session);
        if let Some(prep) = guard.preps.get_mut(&session) {
            prep.active = false;
            prep.attempt_id.clear();
            prep.status = PrepStatus::Stopped;
            prep.reason = Some("cancelled".to_owned());
        }
    }
    save(app).await?;
    Ok(json!({"seat": parsed.seat, "state": "cancelled"}))
}

pub(crate) async fn status(app: &App, body: &[u8]) -> Result<Value, Error> {
    let parsed: SeatBody =
        serde_json::from_slice(body).map_err(|err| Error::Message(err.to_string()))?;
    let seat = find_seat(app, &parsed.seat).await?;
    super::apply::deliver_seat(app, &seat.id.to_string()).await?;
    let guard = app.compaction.lock().await;
    Ok(status_value(&guard, &seat, None, None))
}

/// Drop a preparation whose attempt is done, aborted, failed, or cancelled.
pub(super) fn release_finished(runtime: &mut Runtime) -> bool {
    let sessions = runtime.preps.keys().cloned().collect::<Vec<_>>();
    let mut changed = false;
    for session in sessions {
        if release_one(runtime, &session) {
            changed = true;
        }
    }
    changed
}

fn release_one(runtime: &mut Runtime, session: &str) -> bool {
    let Some(prep) = runtime.preps.get(session) else {
        return false;
    };
    if !prep.active {
        return false;
    }
    let attempt_id = prep.attempt_id.clone();
    if attempt_id.is_empty() {
        return false;
    }
    let Some(stage) = terminal_stage(runtime, &attempt_id) else {
        return false;
    };
    if let Some(prep) = runtime.preps.get_mut(session) {
        prep.active = false;
        prep.attempt_id.clear();
    }
    runtime.enforcer.note_settled(session, stage);
    true
}

/// A failed compact left the prep stopped. The next one may start.
fn stopped_failure(runtime: &Runtime, session: &str, prep: &PreparationState) -> bool {
    prep.active
        && prep.status == PrepStatus::Stopped
        && runtime
            .enforcer
            .manual_stage(session)
            .is_some_and(|stage| !stage.is_open())
}

fn terminal_stage(runtime: &Runtime, attempt_id: &str) -> Option<ManualCompactionStage> {
    runtime.book.attempts().iter().rev().find_map(|attempt| {
        if attempt.id.as_str() != attempt_id {
            return None;
        }
        match &attempt.state {
            AttemptState::Done => Some(ManualCompactionStage::Done),
            AttemptState::Aborted(_) | AttemptState::Failed(_) => {
                Some(ManualCompactionStage::Failed)
            }
            AttemptState::Cancelled => Some(ManualCompactionStage::Cancelled),
            AttemptState::Requested
            | AttemptState::PreCompactSeen { .. }
            | AttemptState::Completed { .. }
            | AttemptState::RestoreSent => None,
        }
    })
}

pub(super) fn expire_preps(runtime: &mut Runtime, now: u64) {
    let sessions = runtime.preps.keys().cloned().collect::<Vec<_>>();
    for session in sessions {
        let Some(stored) = runtime.preps.get(&session).cloned() else {
            continue;
        };
        if !stored.active || stored.deadline_ms.is_none_or(|deadline| now < deadline) {
            continue;
        }
        if matches!(stored.status, PrepStatus::Stopped | PrepStatus::CompactSent) {
            continue;
        }
        let input = manual_input(
            &session,
            stored.attempt_id.as_str(),
            false,
            false,
            SeatId::new(),
            PathBuf::from("/"),
        );
        let outcome = runtime.enforcer.decide_manual(&input, &stored, now);
        if matches!(
            outcome,
            EnforcerOutcome::Skip(
                EnforcerSkipReason::PreparationIncomplete | EnforcerSkipReason::PreparationStopped
            )
        ) && let Some(prep) = runtime.preps.get_mut(&session)
        {
            // The attempt failed. Leaving it active makes the next compact
            // return `preparation_stopped` instead of starting again.
            prep.active = false;
            prep.attempt_id.clear();
            prep.status = PrepStatus::Stopped;
            prep.reason = Some(
                EnforcerSkipReason::PreparationIncomplete
                    .as_str()
                    .to_owned(),
            );
        }
    }
}

async fn drive(app: &App, seat: &Seat, body: &TriggerBody, ready: bool) -> Result<Value, Error> {
    let mut compact_sent = false;
    for _ in 0..2 {
        let step = plan(app, seat, body, ready).await;
        match step.text {
            Some(text) if step.compact => {
                save(app).await?;
                // `/compact` is not a user turn, so the transcript cannot confirm it.
                // Enter still happened: NotVerified and Contaminated count as sent.
                match submit_text(app, seat.name.as_str(), &text).await {
                    Ok(_) => {}
                    Err(Error::Adapter(err)) if compact_entered(&err) => {}
                    Err(err) => return Err(err),
                }
                compact_sent = true;
                break;
            }
            Some(text) => match submit_text(app, seat.name.as_str(), &text).await {
                Ok(_) => {
                    mark_paste_accepted(app, seat.name.as_str()).await;
                    if !body.skip_map {
                        break;
                    }
                }
                Err(err) if before_enter(&err) => {
                    release_failed_prep(app, seat, &err).await?;
                    return Err(err);
                }
                // Enter landed, but the transcript is not the prep we sent.
                // Fail the attempt so it cannot later be recorded as `done`.
                Err(err) => {
                    fail_unconfirmed_prep(app, seat, &err).await?;
                    return Err(err);
                }
            },
            None => return finish_skip(app, seat, step.reason).await,
        }
    }
    save(app).await?;
    let guard = app.compaction.lock().await;
    Ok(status_value(&guard, seat, Some(compact_sent), None))
}

struct Step {
    text: Option<String>,
    compact: bool,
    reason: Option<EnforcerSkipReason>,
}

async fn plan(app: &App, seat: &Seat, body: &TriggerBody, ready: bool) -> Step {
    let mut guard = app.compaction.lock().await;
    let session = seat.name.as_str();
    let stored = guard.prep(session);
    // A stopped prep whose manual stage already failed must not block the next
    // compact, and must not resume into `/compact`.
    let prep = if stopped_failure(&guard, session, &stored) {
        PreparationState::idle()
    } else {
        stored
    };
    let attempt_id = if !prep.active || prep.attempt_id.is_empty() {
        guard.book.next_manual_id(&seat.id.to_string()).0
    } else {
        prep.attempt_id.clone()
    };
    let home = app.home.clone();
    let input = EnforcerInput {
        seat: seat.id,
        session: session.to_owned(),
        runtime: seat.runtime,
        used_percent: body.used_percent.or(Some(0)),
        sample_at_ms: Some(now_ms()),
        threshold: body
            .threshold
            .as_ref()
            .map_or_else(|| Threshold::some(70), Threshold::parse),
        enabled: true,
        ready,
        cwd: seat.cwd.clone(),
        transcript_path: None,
        session_id: None,
        compact_instruction: body.instruction.clone(),
        pre_compact_instruction: None,
        rig_home: home,
        attempt_id: attempt_id.clone(),
        skip_map: body.skip_map,
    };
    let outcome = guard.enforcer.decide_manual(&input, &prep, now_ms());
    if matches!(
        outcome,
        EnforcerOutcome::Skip(EnforcerSkipReason::SendFailed)
    ) {
        guard.book.fail_unsent(
            &seat.id.to_string(),
            AttemptId(attempt_id.clone()),
            "send_failed",
            now_ms(),
        );
        guard
            .enforcer
            .note_settled(session, ManualCompactionStage::Failed);
    }
    let text = gated_submit(ready, &outcome);
    let compact = matches!(outcome, EnforcerOutcome::SendCompact(_));
    if compact {
        guard.book.request(
            &seat.id.to_string(),
            AttemptId(attempt_id.clone()),
            now_ms(),
        );
    }
    runtime_plan(&mut guard, session, &input);
    remember(
        &mut guard,
        session,
        &attempt_id,
        &outcome,
        &prep,
        body.skip_map,
    );
    Step {
        text,
        compact,
        reason: match &outcome {
            EnforcerOutcome::Skip(reason) => Some(*reason),
            EnforcerOutcome::SendPrep(_)
            | EnforcerOutcome::SendCompact(_)
            | EnforcerOutcome::SendRestore(_) => None,
        },
    }
}

fn runtime_plan(runtime: &mut Runtime, session: &str, input: &EnforcerInput) {
    runtime.set_plan(
        session,
        CompactPlan {
            instruction: input.compact_instruction.clone(),
            used_percent: input.used_percent,
            threshold: input.threshold,
        },
    );
}

pub(super) async fn deliver_prepared(app: &App) -> Result<(), Error> {
    let sessions = {
        let guard = app.compaction.lock().await;
        guard
            .preps
            .iter()
            .filter(|(_, prep)| {
                prep.active
                    && prep.paste_accepted
                    && prep.map_complete
                    && matches!(prep.status, PrepStatus::Waiting)
            })
            .map(|(session, _)| session.clone())
            .collect::<Vec<_>>()
    };
    for session in sessions {
        let Ok(seat) = find_seat(app, &session).await else {
            continue;
        };
        if !super::apply::seat_is_ready(app, &seat).await {
            continue;
        }
        send_prepared(app, &seat).await?;
    }
    Ok(())
}

async fn send_prepared(app: &App, seat: &Seat) -> Result<(), Error> {
    let session = seat.name.as_str();
    let text = {
        let mut guard = app.compaction.lock().await;
        let prep = guard.prep(session);
        let Some(plan) = guard.plan(session).cloned() else {
            return Ok(());
        };
        let input = prepared_input(seat, &prep, &plan, &app.home);
        let outcome = match prep.mode {
            PrepMode::Manual => guard.enforcer.decide_manual(&input, &prep, now_ms()),
            PrepMode::Automatic => guard.enforcer.decide(&input, &prep, now_ms()),
        };
        let Some(text) = gated_submit(true, &outcome) else {
            return Ok(());
        };
        if !matches!(outcome, EnforcerOutcome::SendCompact(_)) {
            return Ok(());
        }
        let open = guard
            .book
            .for_seat(&seat.id.to_string())
            .into_iter()
            .any(|attempt| {
                attempt.managed
                    && matches!(
                        attempt.state,
                        crewmux_health::compaction::AttemptState::Requested
                    )
            });
        if !open {
            guard
                .book
                .request(&seat.id.to_string(), AttemptId(prep.attempt_id), now_ms());
        }
        text
    };
    if !compact_landed(submit_text(app, session, &text).await, session) {
        return Ok(());
    }
    {
        let mut guard = app.compaction.lock().await;
        if let Some(prep) = guard.preps.get_mut(session) {
            prep.status = PrepStatus::CompactSent;
            prep.map_complete = true;
        }
    }
    save(app).await
}

fn compact_landed(result: Result<Value, Error>, session: &str) -> bool {
    match result {
        Ok(_) => true,
        Err(Error::Adapter(err)) if compact_entered(&err) => true,
        Err(Error::Adapter(
            crewmux_adapters::Error::NotReady | crewmux_adapters::Error::PasteMismatch,
        )) => false,
        Err(err) => {
            tracing::warn!(session, %err, "compact submit failed");
            false
        }
    }
}

/// Paste and readiness failures happen before Enter. `NotVerified` and
/// `Contaminated` mean Enter already landed.
fn before_enter(err: &Error) -> bool {
    !matches!(
        err,
        Error::Adapter(
            crewmux_adapters::Error::NotVerified | crewmux_adapters::Error::Contaminated
        )
    )
}

fn prep_fail_reason(err: &Error) -> &'static str {
    match err {
        Error::Adapter(crewmux_adapters::Error::PasteMismatch) => "paste_mismatch",
        Error::Store(_)
        | Error::Domain(_)
        | Error::Mux(_)
        | Error::Adapter(_)
        | Error::Spec(_)
        | Error::Io(_)
        | Error::Message(_) => "send_failed",
    }
}

/// The prep paste never landed. Drop it so the next compact is not `preparation_pending`.
async fn release_failed_prep(app: &App, seat: &Seat, err: &Error) -> Result<(), Error> {
    settle_failed_prep(app, seat, prep_fail_reason(err)).await
}

/// The prep was submitted and the transcript did not match it.
async fn fail_unconfirmed_prep(app: &App, seat: &Seat, err: &Error) -> Result<(), Error> {
    settle_failed_prep(app, seat, &err.to_string()).await
}

async fn settle_failed_prep(app: &App, seat: &Seat, reason: &str) -> Result<(), Error> {
    let session = seat.name.as_str();
    {
        let mut guard = app.compaction.lock().await;
        let attempt_id = guard.prep(session).attempt_id;
        if let Some(prep) = guard.preps.get_mut(session) {
            prep.active = false;
            prep.status = PrepStatus::Stopped;
            prep.paste_accepted = false;
            prep.reason = Some(reason.to_owned());
            prep.attempt_id.clear();
        }
        if !attempt_id.is_empty() {
            guard.book.fail_unsent(
                &seat.id.to_string(),
                AttemptId(attempt_id),
                reason,
                now_ms(),
            );
        }
        guard
            .enforcer
            .note_settled(session, ManualCompactionStage::Failed);
    }
    save(app).await
}

async fn mark_paste_accepted(app: &App, session: &str) {
    let mut guard = app.compaction.lock().await;
    if let Some(prep) = guard.preps.get_mut(session) {
        prep.paste_accepted = true;
    }
}

fn compact_entered(err: &crewmux_adapters::Error) -> bool {
    matches!(
        err,
        crewmux_adapters::Error::NotVerified | crewmux_adapters::Error::Contaminated
    )
}

fn prepared_input(
    seat: &Seat,
    prep: &PreparationState,
    plan: &CompactPlan,
    home: &std::path::Path,
) -> EnforcerInput {
    EnforcerInput {
        seat: seat.id,
        session: seat.name.as_str().to_owned(),
        runtime: seat.runtime,
        used_percent: plan.used_percent.or(Some(0)),
        sample_at_ms: Some(now_ms()),
        threshold: plan.threshold,
        enabled: true,
        ready: true,
        cwd: seat.cwd.clone(),
        transcript_path: None,
        session_id: None,
        compact_instruction: plan.instruction.clone(),
        pre_compact_instruction: None,
        rig_home: home.to_path_buf(),
        attempt_id: prep.attempt_id.clone(),
        skip_map: false,
    }
}

fn remember(
    runtime: &mut Runtime,
    session: &str,
    attempt_id: &str,
    outcome: &EnforcerOutcome,
    previous: &PreparationState,
    skip_map: bool,
) {
    let mut prep = previous.clone();
    attempt_id.clone_into(&mut prep.attempt_id);
    prep.mode = PrepMode::Manual;
    match outcome {
        EnforcerOutcome::SendPrep(_) => {
            prep.active = true;
            prep.status = PrepStatus::Waiting;
            prep.map_complete = skip_map;
            prep.paste_accepted = false;
            prep.reason = None;
            prep.deadline_ms = Some(now_ms().saturating_add(wait_ms()));
        }
        EnforcerOutcome::Skip(EnforcerSkipReason::SendFailed) => {
            prep.active = false;
            prep.attempt_id.clear();
            prep.status = PrepStatus::Stopped;
            prep.paste_accepted = false;
            prep.reason = Some(EnforcerSkipReason::SendFailed.as_str().to_owned());
        }
        EnforcerOutcome::SendCompact(_) => {
            prep.active = true;
            prep.status = PrepStatus::CompactSent;
            prep.map_complete = true;
        }
        EnforcerOutcome::Skip(EnforcerSkipReason::PreparationPending) => {
            prep.active = true;
            prep.status = PrepStatus::Waiting;
        }
        // An in-flight compact stays `CompactSent`. Marking it stopped is what
        // made the following compact return `preparation_stopped`.
        EnforcerOutcome::Skip(EnforcerSkipReason::AlreadyInProgress)
        | EnforcerOutcome::SendRestore(_) => {}
        EnforcerOutcome::Skip(_) => {
            prep.active = false;
            prep.attempt_id.clear();
            prep.status = PrepStatus::Stopped;
        }
    }
    runtime.set_prep(session, prep);
}

async fn finish_skip(
    app: &App,
    seat: &Seat,
    reason: Option<EnforcerSkipReason>,
) -> Result<Value, Error> {
    save(app).await?;
    let guard = app.compaction.lock().await;
    let reason = reason.map_or("skipped", EnforcerSkipReason::as_str);
    let value = status_value(&guard, seat, None, Some(reason));
    if reason == "preparation_pending" {
        return Ok(value);
    }
    Err(Error::Message(reason.to_owned()))
}

fn status_value(
    runtime: &Runtime,
    seat: &Seat,
    compact_sent: Option<bool>,
    reason: Option<&str>,
) -> Value {
    let key = seat.id.to_string();
    let mut attempts = runtime
        .book
        .for_seat(&key)
        .into_iter()
        .map(crewmux_health::compaction::summary_value)
        .collect::<Vec<_>>();
    if attempts.is_empty() {
        let prep = runtime.prep(seat.name.as_str());
        if prep.active && !prep.attempt_id.is_empty() {
            attempts.push(prep_row(&prep, &key));
        }
    }
    let mut value = json!({
        "seat": seat.name.as_str(),
        "seat_key": key,
        "stage": runtime.enforcer.manual_stage(seat.name.as_str()).map(crewmux_health::compaction::ManualCompactionStage::as_str),
        "attempts": attempts,
    });
    if let Some(object) = value.as_object_mut() {
        if let Some(sent) = compact_sent {
            object.insert("compact_sent".to_owned(), json!(sent));
        }
        if let Some(reason) = reason {
            object.insert("reason".to_owned(), json!(reason));
        }
    }
    value
}

fn prep_row(prep: &PreparationState, seat_key: &str) -> Value {
    let state = match prep.status {
        PrepStatus::Sending | PrepStatus::Waiting => "preparing",
        PrepStatus::CompactSent => "compacting",
        PrepStatus::Stopped => "stopped",
    };
    json!({
        "attempt_id": prep.attempt_id,
        "seat_key": seat_key,
        "trigger": "manual",
        "managed": true,
        "state": state,
        "abort_reason": prep.reason,
        "summary_sha": Value::Null,
        "summary_len": Value::Null,
        "confirmed": false,
    })
}

fn manual_input(
    session: &str,
    attempt_id: &str,
    ready: bool,
    skip_map: bool,
    seat: SeatId,
    home: PathBuf,
) -> EnforcerInput {
    EnforcerInput {
        seat,
        session: session.to_owned(),
        runtime: RuntimeKind::ClaudeCode,
        used_percent: Some(0),
        sample_at_ms: Some(now_ms()),
        threshold: Threshold::some(70),
        enabled: true,
        ready,
        cwd: None,
        transcript_path: None,
        session_id: None,
        compact_instruction: String::new(),
        pre_compact_instruction: None,
        rig_home: home,
        attempt_id: attempt_id.to_owned(),
        skip_map,
    }
}

fn wait_ms() -> u64 {
    u64::try_from(MANUAL_PREP_WAIT.as_millis()).unwrap_or(0)
}
