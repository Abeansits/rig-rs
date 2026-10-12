//! `rig up`: plan, launch stub seats, and wait until each one settles.

use std::collections::BTreeMap;
use std::time::Duration;

use crewmux_adapters::{BudgetOverrides, LaunchBudget, LaunchOpts, LaunchOutcome, binding_for};
use crewmux_core::{
    LaunchApply, LifecycleService, PlanIds, RigRepo, SeatRepo, SeatState, Timestamp,
};
use crewmux_spec::{RigSpec, SeatSpec};
use crewmux_store::{clear_rig, put_seat_token};
use crewmux_types::{ReadinessCode, RigId, SEAT_ID_ENV, SEAT_TOKEN_ENV, SeatId, SessionId};
use serde_json::{Value, json};

use crate::db::{read, write};
use crate::error::Error;
use crate::state::{App, join_err, now_ms};
use crate::view::{event_value, seat_value, status_of};

struct Up {
    rig_id: RigId,
    rig_name: String,
    events: Vec<Value>,
    seats: Vec<crewmux_core::Seat>,
    sessions: Vec<SessionId>,
}

pub(crate) async fn bring_up(app: &App, yaml: &str) -> Result<Value, Error> {
    let spec = crewmux_spec::parse(yaml)?;
    let _guard = app.lock_rig(spec.name.as_str()).await;
    if let Some(body) = already_running(app, &spec).await? {
        return Ok(body);
    }
    let mut up = insert_plan(app, &spec).await?;
    launch_seats(app, &mut up, &spec.seats).await?;
    crate::settle::settle(app, &mut up.seats, &mut up.events).await?;
    let rig_id = up.rig_id;
    let seats = read(&app.store, move |conn| conn.list_by_rig(&rig_id)).await?;
    Ok(json!({
        "status": status_of(&seats),
        "rig": up.rig_name,
        "seats": seats.iter().map(|seat| seat_value(seat, None)).collect::<Vec<_>>(),
        "events": up.events,
    }))
}

async fn already_running(app: &App, spec: &RigSpec) -> Result<Option<Value>, Error> {
    let name = spec.name.clone();
    let found = read(&app.store, move |conn| {
        let Some(rig) = conn.get_by_name(&name)? else {
            return Ok(None);
        };
        let seats = conn.list_by_rig(&rig.id)?;
        Ok(Some((rig.id, seats)))
    })
    .await?;
    let Some((rig_id, seats)) = found else {
        return Ok(None);
    };
    let live = seats
        .iter()
        .any(|seat| !matches!(seat.state, SeatState::Stopped | SeatState::Dead));
    if !live {
        write(&app.store, move |conn| clear_rig(conn, &rig_id)).await?;
        return Ok(None);
    }
    if seats.iter().any(|seat| seat.state == SeatState::Dead) {
        return relaunch_dead(app, spec.name.as_str(), &seats)
            .await
            .map(Some);
    }
    Ok(Some(json!({
        "status": "already_running",
        "rig": spec.name.as_str(),
        "seats": seats.iter().map(|seat| seat_value(seat, None)).collect::<Vec<_>>(),
        "events": [],
    })))
}

/// Relaunch dead seats in place. Ready seats keep their ids and sessions.
async fn relaunch_dead(
    app: &App,
    rig_name: &str,
    seats: &[crewmux_core::Seat],
) -> Result<Value, Error> {
    let mut events = Vec::new();
    let mut launching = Vec::new();
    for seat in seats.iter().filter(|seat| seat.state == SeatState::Dead) {
        clear_leftover_session(app, rig_name, seat.name.as_str()).await?;
        let next = launch_one(
            app,
            seat,
            SessionId::new(),
            rig_name,
            &mut events,
            SeatHints::default(),
            crate::resume::LaunchExtra::initial(seat.name.as_str()),
        )
        .await?;
        launching.push(next);
    }
    crate::settle::settle(app, &mut launching, &mut events).await?;
    let rig_id = seats
        .first()
        .map(|seat| seat.rig_id)
        .ok_or_else(|| Error::Message("rig has no seats".to_owned()))?;
    let stored = read(&app.store, move |conn| conn.list_by_rig(&rig_id)).await?;
    Ok(json!({
        "status": status_of(&stored),
        "rig": rig_name,
        "seats": stored.iter().map(|seat| seat_value(seat, None)).collect::<Vec<_>>(),
        "events": events,
    }))
}

async fn clear_leftover_session(app: &App, rig_name: &str, seat_name: &str) -> Result<(), Error> {
    let session_name = format!("{seat_name}__{rig_name}");
    if !app.mux.has_session(&session_name).await? {
        return Ok(());
    }
    match app.mux.kill_session(&session_name).await {
        Ok(()) | Err(crewmux_mux::MuxError::NotFound(_)) => Ok(()),
        Err(err) => Err(err.into()),
    }
}

async fn insert_plan(app: &App, spec: &RigSpec) -> Result<Up, Error> {
    let name = spec.name.clone();
    let existing = read(&app.store, move |conn| conn.get_by_name(&name)).await?;
    let fresh = existing.is_none();
    let rig_id = existing.map_or_else(RigId::new, |rig| rig.id);
    let sessions: Vec<SessionId> = spec.seats.iter().map(|_| SessionId::new()).collect();
    let ids = PlanIds {
        rig: rig_id,
        seats: spec.seats.iter().map(|_| SeatId::new()).collect(),
        sessions: sessions.clone(),
    };
    let now = now_ts();
    let plan = LifecycleService::plan_up(spec, &ids, now)?;
    let mut events = Vec::new();
    if fresh {
        for event in &plan.events {
            events.push(event_value(event, None));
        }
    }
    let stored_events = if fresh { plan.events } else { Vec::new() };
    let rig = plan.rig.clone();
    let seats = plan.seats.clone();
    write(&app.store, move |conn| {
        RigRepo::upsert(conn, &rig)?;
        for seat in &seats {
            SeatRepo::upsert(conn, seat)?;
        }
        for event in &stored_events {
            crewmux_core::EventLog::append(conn, event)?;
        }
        Ok(())
    })
    .await?;
    Ok(Up {
        rig_id,
        rig_name: spec.name.as_str().to_owned(),
        events,
        seats: plan.seats,
        sessions,
    })
}

async fn launch_seats(app: &App, up: &mut Up, specs: &[SeatSpec]) -> Result<(), Error> {
    let rig_name = up.rig_name.clone();
    let mut launched = Vec::with_capacity(up.seats.len());
    for ((seat, session), spec) in up
        .seats
        .iter()
        .zip(up.sessions.iter().copied())
        .zip(specs.iter())
    {
        let hints = SeatHints {
            auto_trust: spec.auto_trust,
            budget: budget_from(spec),
        };
        let extra = crate::resume::LaunchExtra::initial(seat.name.as_str());
        let next = launch_one(app, seat, session, &rig_name, &mut up.events, hints, extra).await?;
        launched.push(next);
    }
    up.seats = launched;
    Ok(())
}

#[derive(Default)]
struct SeatHints {
    auto_trust: bool,
    budget: Option<LaunchBudget>,
}

fn budget_from(spec: &SeatSpec) -> Option<LaunchBudget> {
    let launch = spec.launch.as_ref()?;
    Some(LaunchBudget::with_overrides(BudgetOverrides {
        first_screen: launch.first_screen.map(Duration::from_millis),
        ready_after_last_gate: launch.ready_after_last_gate.map(Duration::from_millis),
        inconclusive_stable: launch.inconclusive_stable.map(Duration::from_millis),
        overall: launch.overall.map(Duration::from_millis),
        poll: launch.poll.map(Duration::from_millis),
    }))
}

pub(crate) async fn relaunch(
    app: &App,
    seat: &crewmux_core::Seat,
    rig_name: &str,
    extra: crate::resume::LaunchExtra,
    events: &mut Vec<Value>,
) -> Result<crewmux_core::Seat, Error> {
    launch_one(
        app,
        seat,
        SessionId::new(),
        rig_name,
        events,
        SeatHints::default(),
        extra,
    )
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "session, hints, and restore extras are separate launch inputs"
)]
async fn launch_one(
    app: &App,
    seat: &crewmux_core::Seat,
    session: SessionId,
    rig_name: &str,
    events: &mut Vec<Value>,
    hints: SeatHints,
    extra: crate::resume::LaunchExtra,
) -> Result<crewmux_core::Seat, Error> {
    let cwd = seat_cwd(seat).await?;
    let session_name = format!("{}__{rig_name}", seat.name.as_str());
    let mut binding = binding_for(seat.id, cwd, &session_name);
    binding.posture = extra.posture;
    binding.model.clone_from(&seat.model);
    let Some(adapter) = app.registry.get(seat.runtime) else {
        return fail_seat(
            app,
            seat,
            "runtime is not available in this slice",
            "unsupported_runtime",
        )
        .await;
    };
    let mut env = BTreeMap::new();
    if let Some(script) = crate::resume::script_for(app, seat.name.as_str()) {
        env.insert("RIG_STUB_SCRIPT".to_owned(), script);
    }
    env.insert(
        "RIG_STUB_POSTURE".to_owned(),
        crewmux_durability::posture_token(extra.posture).to_owned(),
    );
    if let Some(path) = &extra.replay {
        env.insert("RIG_STUB_REPLAY".to_owned(), path.display().to_string());
    }
    let seat_token = SeatId::new().to_string();
    env.insert(SEAT_ID_ENV.to_owned(), seat.id.to_string());
    env.insert(SEAT_TOKEN_ENV.to_owned(), seat_token.clone());
    let posture = extra.posture;
    let outcome = adapter
        .launch(
            &binding,
            LaunchOpts {
                session_name,
                env,
                resume: extra.resume,
                fork: false,
                auto_trust: hints.auto_trust,
                budget: hints.budget,
                claude_bin: None,
            },
        )
        .await?;
    match outcome {
        LaunchOutcome::Ok { applied, .. } => {
            let updated = store_started(app, seat, session, applied, events, &seat_token).await?;
            crate::resume::remember(app, &updated, posture).await?;
            Ok(updated)
        }
        LaunchOutcome::Failed { error, .. } => {
            fail_seat(app, seat, &error, attention_code(&error)).await
        }
        LaunchOutcome::LoginRequired { kind, evidence } => {
            login_required_seat(app, seat, kind, evidence).await
        }
    }
}

async fn login_required_seat(
    app: &App,
    seat: &crewmux_core::Seat,
    kind: crewmux_types::LoginKind,
    evidence: String,
) -> Result<crewmux_core::Seat, Error> {
    let mut updated = seat.clone();
    LifecycleService::mark_login_required(&mut updated, kind, evidence);
    let saved = updated.clone();
    write(&app.store, move |conn| SeatRepo::upsert(conn, &saved)).await?;
    Ok(updated)
}

async fn seat_cwd(seat: &crewmux_core::Seat) -> Result<std::path::PathBuf, Error> {
    if let Some(path) = &seat.cwd {
        return Ok(path.clone());
    }
    tokio::task::spawn_blocking(std::env::current_dir)
        .await
        .map_err(join_err)?
        .map_err(Error::from)
}

async fn store_started(
    app: &App,
    seat: &crewmux_core::Seat,
    session: SessionId,
    applied: crewmux_adapters::AppliedLaunch,
    events: &mut Vec<Value>,
    token: &str,
) -> Result<crewmux_core::Seat, Error> {
    let mut updated = seat.clone();
    let now = now_ts();
    let pane_id = applied.pane.pane_id.clone();
    let tmux_session = applied.tmux_session.clone();
    let event = LifecycleService::apply_launch_result(
        &mut updated,
        LaunchApply::Started {
            session,
            tmux_session: tmux_session.clone(),
            pane_id: pane_id.clone(),
        },
        now,
    );
    let saved = updated.clone();
    let name = saved.name.as_str().to_owned();
    let stored = event.clone();
    let token = token.to_owned();
    write(&app.store, move |conn| {
        SeatRepo::upsert(conn, &saved)?;
        put_seat_token(conn, &saved.id, &token)?;
        crewmux_core::SessionRepo::insert(
            conn,
            &session,
            &saved.id,
            &tmux_session,
            pane_id.as_deref(),
            now,
        )?;
        if let Some(event) = &stored {
            crewmux_core::EventLog::append(conn, event)?;
        }
        Ok(())
    })
    .await?;
    if let Some(event) = &event {
        events.push(event_value(event, Some(&name)));
    }
    Ok(updated)
}

async fn fail_seat(
    app: &App,
    seat: &crewmux_core::Seat,
    reason: &str,
    code: &str,
) -> Result<crewmux_core::Seat, Error> {
    let mut updated = seat.clone();
    drop(LifecycleService::apply_launch_result(
        &mut updated,
        LaunchApply::Failed {
            code: ReadinessCode::parse(code),
            reason: reason.to_owned(),
        },
        now_ts(),
    ));
    let saved = updated.clone();
    write(&app.store, move |conn| SeatRepo::upsert(conn, &saved)).await?;
    Ok(updated)
}

pub(crate) fn attention_code(error: &str) -> &'static str {
    let lower = error.to_ascii_lowercase();
    if lower.contains("permission") || lower.contains("not permitted") {
        "permission_denied"
    } else if lower.contains("no such file") || lower.contains("not found") {
        "not_found"
    } else if lower.contains("codex_home_missing") || lower.contains("codex home") {
        "codex_home_missing"
    } else if lower.contains("cwd") || lower.contains("directory") {
        "invalid_cwd"
    } else {
        "launch_failed"
    }
}

pub(crate) fn now_ts() -> Timestamp {
    Timestamp::from_millis(i64::try_from(now_ms()).unwrap_or(0))
}
