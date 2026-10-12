//! `rig compact`: prep, then `/compact`, only through submit when the seat is `Ready`.

use std::path::PathBuf;

use crewmux_core::Seat;
use crewmux_health::compaction::{
    AttemptId, EnforcerInput, EnforcerOutcome, EnforcerSkipReason, MANUAL_PREP_WAIT, PrepMode,
    PrepStatus, PreparationState, Threshold, gated_submit,
};
use crewmux_types::{RuntimeKind, SeatId};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::Error;
use crate::query::find_seat;
use crate::state::{App, now_ms};
use crate::submit::submit_text;

use super::Runtime;
use super::persist::save;

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
                submit_text(app, seat.name.as_str(), &text).await?;
                compact_sent = true;
                break;
            }
            Some(text) => {
                submit_text(app, seat.name.as_str(), &text).await?;
                if !body.skip_map {
                    break;
                }
            }
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
    let prep = guard.prep(session);
    let attempt_id = if prep.attempt_id.is_empty() {
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
    let text = gated_submit(ready, &outcome);
    let compact = matches!(outcome, EnforcerOutcome::SendCompact(_));
    if compact {
        guard.book.request(
            &seat.id.to_string(),
            AttemptId(attempt_id.clone()),
            now_ms(),
        );
    }
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
        EnforcerOutcome::SendPrep(_) | EnforcerOutcome::Skip(EnforcerSkipReason::SendFailed) => {
            prep.active = true;
            prep.status = PrepStatus::Waiting;
            prep.map_complete = skip_map;
            if prep.deadline_ms.is_none() {
                prep.deadline_ms = Some(now_ms().saturating_add(wait_ms()));
            }
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
        EnforcerOutcome::Skip(_) => {
            prep.active = true;
            prep.status = PrepStatus::Stopped;
        }
        EnforcerOutcome::SendRestore(_) => {}
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
    if reason == "send_failed" || reason == "preparation_pending" {
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
    let attempts = runtime
        .book
        .for_seat(&key)
        .into_iter()
        .map(crewmux_health::compaction::summary_value)
        .collect::<Vec<_>>();
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
