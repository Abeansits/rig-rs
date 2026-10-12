//! Relaunch a rig in wave order and reconcile in-flight deliveries.

use std::path::PathBuf;
use std::sync::Arc;

use crewmux_adapters::{ResumeToken, binding_for};
use crewmux_core::{DomainEvent, Seat, SeatRepo, SeatState};
use crewmux_durability::{
    InFlightDelivery, LaunchEdge, ReconcileAction, RuntimeEvidence, SeatRestoreOutcome,
    TruthReconcile, clamp_posture, launch_waves, plan_reconcile, reconcile_runtime_truth,
    renudge_text, rollup,
};
use crewmux_types::{LaunchPosture, ReadinessCode, RigId, SeatId};
use serde_json::{Value, json};

use crate::deliver::{Outbound, deliver};
use crate::direct::{HomeProbe, delivery_confirmed};
use crate::error::Error;
use crate::launch::{now_ts, relaunch};
use crate::notify::emit;
use crate::resume::{LaunchExtra, recorded_posture, replay_path, resume_path};
use crate::snapshot::load_rig;
use crate::state::{App, join_err};
use crate::turns::envelope_turns;

/// Restore one rig.
///
/// # Errors
///
/// Returns [`Error`] when the snapshot is refused or the rig is missing.
pub(crate) async fn restore_rig(
    app: &App,
    rig_name: &str,
    snapshot: Option<&str>,
    requested: LaunchPosture,
) -> Result<Value, Error> {
    if app.take_cancel_on_restore() {
        app.request_cart_cancel();
    }
    if let Some(id) = snapshot {
        crate::snapshot::apply(app, rig_name, id).await?;
    }
    let (rig_id, seats) = load_rig(app, rig_name).await?;
    let edges = edges_of(app, &rig_id).await?;
    let waves = order_waves(&seats, &edges)?;
    let mut reports = Vec::new();
    let mut outcomes = Vec::new();
    for wave in &waves {
        for id in wave {
            let Some(seat) = seats.iter().find(|seat| seat.id == *id) else {
                continue;
            };
            let (report, outcome) = restore_one(app, seat, rig_name, requested).await?;
            reports.push(report);
            outcomes.push(outcome);
        }
    }
    let actions = reconcile_deliveries(app, &rig_id, rig_name, &seats).await?;
    let outcome = rollup(&outcomes);
    emit(
        app,
        DomainEvent::RestoreCompleted {
            rig: rig_id,
            outcome: outcome.as_str().to_owned(),
            at: now_ts(),
        },
    )
    .await;
    Ok(json!({
        "rig": rig_name,
        "outcome": outcome.as_str(),
        "waves": wave_names(&seats, &waves),
        "seats": reports,
        "reconcile": actions,
    }))
}

async fn restore_one(
    app: &App,
    seat: &Seat,
    rig_name: &str,
    requested: LaunchPosture,
) -> Result<(Value, SeatRestoreOutcome), Error> {
    let recorded = recorded_posture(app, seat).await?;
    let posture = clamp_posture(recorded, requested);
    let had = discover(app, seat, rig_name).await?;
    let alive = session_alive(app, seat).await?;
    let (seat, launched_resume) = if alive && matches!(seat.state, SeatState::Ready) {
        (seat.clone(), had.is_some())
    } else {
        let replay = if had.is_some() {
            write_replay(app, seat, rig_name).await?
        } else {
            None
        };
        let extra = LaunchExtra {
            posture,
            resume: had.clone(),
            replay,
        };
        let mut events = Vec::new();
        let mut launched = relaunch(app, seat, rig_name, extra, &mut events).await?;
        crate::settle::settle(app, std::slice::from_mut(&mut launched), &mut events).await?;
        let id = launched.id;
        let launched = crate::db::read(&app.store, move |conn| conn.get(&id)).await?;
        (launched, had.is_some())
    };
    let outcome = outcome_of(&seat, launched_resume);
    let (outcome, truth) = apply_truth(&seat, outcome, launched_resume, alive || launched_resume);
    Ok((seat_json(&seat, posture, &outcome, truth), outcome))
}

fn outcome_of(seat: &Seat, had_resume: bool) -> SeatRestoreOutcome {
    match &seat.state {
        SeatState::Attention(code) => SeatRestoreOutcome::AttentionRequired(code.clone()),
        SeatState::LoginRequired { .. } => {
            SeatRestoreOutcome::AttentionRequired(ReadinessCode::LoginRequired)
        }
        SeatState::Ready if had_resume => SeatRestoreOutcome::Resumed,
        SeatState::Ready => SeatRestoreOutcome::Fresh,
        SeatState::Dead | SeatState::Stopped | SeatState::Planned | SeatState::Launching => {
            SeatRestoreOutcome::Failed(
                seat.reason
                    .clone()
                    .unwrap_or_else(|| "launch failed".to_owned()),
            )
        }
    }
}

fn apply_truth(
    seat: &Seat,
    outcome: SeatRestoreOutcome,
    had_resume: bool,
    session_up: bool,
) -> (SeatRestoreOutcome, Option<&'static str>) {
    let upgradable = matches!(
        outcome,
        SeatRestoreOutcome::Failed(_) | SeatRestoreOutcome::AttentionRequired(_)
    );
    let evidence = RuntimeEvidence {
        prior: outcome.clone(),
        tmux_alive: session_up,
        binding_matches: seat.tmux_session.is_some(),
        resume_token_used: had_resume,
        pane_usable: matches!(seat.state, SeatState::Ready),
    };
    match reconcile_runtime_truth(&evidence) {
        TruthReconcile::OperatorRecovered => (SeatRestoreOutcome::OperatorRecovered, None),
        TruthReconcile::Noop { reason } if upgradable => (outcome, Some(reason)),
        TruthReconcile::Noop { .. } => (outcome, None),
    }
}

fn seat_json(
    seat: &Seat,
    posture: LaunchPosture,
    outcome: &SeatRestoreOutcome,
    truth: Option<&str>,
) -> Value {
    let attention = match outcome {
        SeatRestoreOutcome::AttentionRequired(code) => Some(code.as_str().to_owned()),
        SeatRestoreOutcome::Failed(reason) => Some(reason.clone()),
        SeatRestoreOutcome::Resumed
        | SeatRestoreOutcome::Fresh
        | SeatRestoreOutcome::OperatorRecovered => None,
    };
    json!({
        "seat": seat.name.as_str(),
        "outcome": outcome.as_str(),
        "attention": attention,
        "posture": crewmux_durability::posture_token(posture),
        "truth": truth,
    })
}

async fn discover(app: &App, seat: &Seat, rig_name: &str) -> Result<Option<ResumeToken>, Error> {
    let Some(adapter) = app.registry.get(seat.runtime) else {
        return Ok(None);
    };
    let cwd = seat.cwd.clone().unwrap_or_default();
    let session = format!("{}__{rig_name}", seat.name.as_str());
    let mut binding = binding_for(seat.id, cwd, &session);
    binding.runtime_opts = json!({"resume_file": resume_path(&app.home, &seat.id)});
    adapter
        .discover_resume_token(&binding)
        .await
        .map_err(Error::from)
}

async fn write_replay(app: &App, seat: &Seat, rig_name: &str) -> Result<Option<PathBuf>, Error> {
    let name = seat.name.as_str().to_owned();
    let rig = rig_name.to_owned();
    let home = app.home.clone();
    let dest = replay_path(&app.home, &seat.id);
    let wrote = tokio::task::spawn_blocking(move || replay_file(&home, &rig, &name, &dest))
        .await
        .map_err(join_err)??;
    Ok(wrote.then_some(replay_path(&app.home, &seat.id)))
}

fn replay_file(
    home: &std::path::Path,
    rig: &str,
    seat: &str,
    dest: &std::path::Path,
) -> Result<bool, Error> {
    let bytes = crate::transcripts::plain_bytes(home, rig, seat)?;
    let turns = envelope_turns(&bytes, 0);
    if turns.is_empty() {
        return Ok(false);
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = turns
        .iter()
        .map(|turn| turn.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(dest, text)?;
    Ok(true)
}

async fn session_alive(app: &App, seat: &Seat) -> Result<bool, Error> {
    match &seat.tmux_session {
        Some(name) => app.mux.has_session(name).await.map_err(Error::from),
        None => Ok(false),
    }
}

fn order_waves(
    seats: &[Seat],
    edges: &[crewmux_store::EdgeRow],
) -> Result<Vec<Vec<SeatId>>, Error> {
    let names: Vec<(SeatId, String)> = seats
        .iter()
        .map(|seat| (seat.id, seat.name.as_str().to_owned()))
        .collect();
    let refs: Vec<(SeatId, &str)> = names
        .iter()
        .map(|(id, name)| (*id, name.as_str()))
        .collect();
    let launch_edges = edges
        .iter()
        .filter_map(|edge| {
            Some(LaunchEdge {
                source: edge.source_id.parse().ok()?,
                target: edge.target_id.parse().ok()?,
                kind: edge.kind.clone(),
            })
        })
        .collect::<Vec<_>>();
    launch_waves(&refs, &launch_edges).map_err(|err| Error::Message(err.to_string()))
}

fn wave_names(seats: &[Seat], waves: &[Vec<SeatId>]) -> Vec<Vec<String>> {
    waves
        .iter()
        .map(|wave| {
            wave.iter()
                .filter_map(|id| seats.iter().find(|seat| seat.id == *id))
                .map(|seat| seat.name.as_str().to_owned())
                .collect()
        })
        .collect()
}

async fn edges_of(app: &App, rig: &RigId) -> Result<Vec<crewmux_store::EdgeRow>, Error> {
    let id = rig.to_string();
    let store = Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || {
        store.read(move |conn| crewmux_store::list_edges(conn, &id))
    })
    .await
    .map_err(join_err)?
    .map_err(Error::from)
}

async fn reconcile_deliveries(
    app: &App,
    rig: &RigId,
    rig_name: &str,
    seats: &[Seat],
) -> Result<Vec<Value>, Error> {
    let planned = plan_for(app, rig, rig_name, seats).await?;
    let mut rendered = Vec::new();
    for action in planned {
        apply_action(app, rig_name, seats, &action).await;
        rendered
            .push(serde_json::to_value(&action).map_err(|err| Error::Message(err.to_string()))?);
    }
    Ok(rendered)
}

async fn plan_for(
    app: &App,
    rig: &RigId,
    rig_name: &str,
    seats: &[Seat],
) -> Result<Vec<ReconcileAction>, Error> {
    let id = rig.to_string();
    let home = app.home.clone();
    let rig_name = rig_name.to_owned();
    let seats = seats.to_vec();
    let store = Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || {
        let inflight = store.read(move |conn| inflight_of(conn, &id))?;
        let probe = HomeProbe::new(home, &rig_name, &seats);
        plan_reconcile(&inflight, &probe)
            .map_err(|err| crewmux_store::Error::Event(err.to_string()))
    })
    .await
    .map_err(join_err)?
    .map_err(Error::from)
}

fn inflight_of(
    conn: &crewmux_store::Connection,
    rig: &str,
) -> Result<Vec<InFlightDelivery>, crewmux_store::Error> {
    let book = crewmux_store::load_book(conn)?;
    let pending = crewmux_messaging::unconfirmed_ids(&book.items());
    let mut items = Vec::new();
    for item in book.items() {
        if pending.iter().any(|id| id == &item.id) {
            items.push(InFlightDelivery::QueueRow {
                qitem: item.id,
                state: item.state,
            });
        }
    }
    for row in crewmux_store::list_direct(conn, rig)? {
        if row.acked {
            continue;
        }
        let Ok(id) = crewmux_types::DeliveryId::new(row.id) else {
            continue;
        };
        let Ok(seat) = row.seat_id.parse() else {
            continue;
        };
        items.push(InFlightDelivery::Direct {
            id,
            seat,
            fingerprint: row.fingerprint,
            body: row.body,
            delivered_at: row.delivered_at_ms,
            cursor: row.cursor.and_then(|value| u64::try_from(value).ok()),
            channel: if row.channel == "startup" {
                crewmux_durability::DeliveryChannel::Startup
            } else {
                crewmux_durability::DeliveryChannel::Direct
            },
        });
    }
    Ok(items)
}

async fn apply_action(app: &App, rig_name: &str, seats: &[Seat], action: &ReconcileAction) {
    match action {
        ReconcileAction::ReNudge(id) => drop(nudge(app, id).await),
        ReconcileAction::Ack(id) => drop(mark_acked(app, id.as_str()).await),
        ReconcileAction::ReSend(id) => drop(resend(app, rig_name, seats, id.as_str()).await),
        ReconcileAction::Escalate(_, _) => {}
    }
}

async fn nudge(app: &App, id: &crewmux_types::QItemId) -> Result<(), Error> {
    let needle = id.clone();
    let store = Arc::clone(&app.store);
    let target = tokio::task::spawn_blocking(move || {
        store.read(move |conn| {
            let book = crewmux_store::load_book(conn)?;
            let found = book.items().into_iter().find(|item| item.id == needle);
            Ok(found.map(|item| item.destination.to_string()))
        })
    })
    .await
    .map_err(join_err)??;
    let Some(target) = target else {
        return Ok(());
    };
    drop(deliver(app, pointer_outbound(&target, &renudge_text(id))).await);
    Ok(())
}

fn pointer_outbound(target: &str, body: &str) -> Outbound {
    Outbound {
        from: "restore@system".to_owned(),
        targets: vec![target.to_owned()],
        rig: None,
        all: false,
        body: body.to_owned(),
        interact: false,
        reason: None,
        verify: false,
        track_direct: false,
    }
}

async fn mark_acked(app: &App, id: &str) -> Result<(), Error> {
    let id = id.to_owned();
    let store = Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || {
        store.write(move |conn| crewmux_store::set_direct_acked(conn, &id))
    })
    .await
    .map_err(join_err)?
    .map_err(Error::from)
}

async fn resend(app: &App, rig_name: &str, seats: &[Seat], id: &str) -> Result<(), Error> {
    let row = load_direct(app, rig_name, id).await?;
    let Some(row) = row else {
        return Ok(());
    };
    let Some(seat) = seats.iter().find(|seat| seat.id.to_string() == row.seat_id) else {
        return Ok(());
    };
    let address = format!("{seat}@{rig_name}", seat = seat.name.as_str());
    let report = deliver(app, pointer_outbound(&address, &row.body)).await;
    let Ok(report) = report else {
        return Ok(());
    };
    if !delivery_confirmed(&report) {
        return Ok(());
    }
    let id = id.to_owned();
    let at = now_ms_i64();
    let store = Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || {
        store.write(move |conn| crewmux_store::set_direct_delivered(conn, &id, at))
    })
    .await
    .map_err(join_err)?
    .map_err(Error::from)
}

async fn load_direct(
    app: &App,
    rig_name: &str,
    id: &str,
) -> Result<Option<crewmux_store::DirectRow>, Error> {
    let (rig_id, _) = load_rig(app, rig_name).await?;
    let rig = rig_id.to_string();
    let id = id.to_owned();
    let store = Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || {
        store.read(move |conn| {
            let rows = crewmux_store::list_direct(conn, &rig)?;
            Ok(rows.into_iter().find(|row| row.id == id))
        })
    })
    .await
    .map_err(join_err)?
    .map_err(Error::from)
}

fn now_ms_i64() -> i64 {
    i64::try_from(crate::state::now_ms()).unwrap_or(0)
}
