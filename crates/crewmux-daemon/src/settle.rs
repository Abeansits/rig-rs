//! Poll stub screens until each launching seat is ready, gated, dead, or escalated.

use std::time::Duration;

use crewmux_adapters::{ReadinessWatch, WatchDecision, default_budget, default_launch_timeout};
use crewmux_core::{LifecycleService, Seat, SeatRepo, SeatState};
use crewmux_mux::{CaptureRange, PaneRef};
use crewmux_types::{ReadinessCode, SeatId};
use serde_json::Value;

use crate::db::{read, write};
use crate::error::Error;
use crate::launch::now_ts;
use crate::state::App;
use crate::view::event_value;

struct Pending {
    id: SeatId,
    binding: crewmux_adapters::NodeBinding,
    watch: ReadinessWatch,
}

pub(crate) async fn settle(
    app: &App,
    seats: &mut [Seat],
    events: &mut Vec<Value>,
) -> Result<(), Error> {
    let mut pending = Vec::new();
    for seat in seats
        .iter()
        .filter(|seat| seat.state == SeatState::Launching)
    {
        let Some(session) = seat.tmux_session.clone() else {
            continue;
        };
        let mut binding =
            crewmux_adapters::binding_for(seat.id, std::path::PathBuf::new(), &session);
        binding.pane = PaneRef {
            session,
            window: Some(0),
            pane: Some(0),
            pane_id: seat.pane_id.clone(),
        };
        let timeout = app
            .registry
            .get(seat.runtime)
            .map_or_else(default_launch_timeout, |adapter| adapter.launch_timeout());
        pending.push(Pending {
            id: seat.id,
            binding,
            watch: ReadinessWatch::new(now_ts(), default_budget(), timeout),
        });
    }
    let started = tokio::time::Instant::now();
    loop {
        let mut open = false;
        for item in &mut pending {
            if !step(app, item, events).await? {
                open = true;
            }
        }
        if !open || started.elapsed() >= default_launch_timeout() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    for item in &pending {
        let id = item.id;
        let seat = read(&app.store, move |conn| conn.get(&id)).await?;
        if seat.state == SeatState::Launching {
            mark_attention(
                app,
                &seat,
                ReadinessCode::Other("launch_timeout".to_owned()),
                "launch timed out",
            )
            .await?;
        }
    }
    Ok(())
}

/// `true` when the seat is no longer launching.
async fn step(app: &App, item: &mut Pending, events: &mut Vec<Value>) -> Result<bool, Error> {
    let id = item.id;
    let seat = read(&app.store, move |conn| conn.get(&id)).await?;
    if seat.state != SeatState::Launching {
        return Ok(true);
    }
    let Some(adapter) = app.registry.get(seat.runtime) else {
        return Ok(true);
    };
    let verdict = adapter.check_ready(&item.binding).await?;
    let capture = app
        .mux
        .capture(&item.binding.pane, CaptureRange::Visible)
        .await?;
    match item.watch.observe(now_ts(), &verdict, &capture) {
        WatchDecision::Pending => Ok(false),
        WatchDecision::Ready => {
            mark_ready(app, &seat, events).await?;
            Ok(true)
        }
        WatchDecision::Attention { code, evidence } => {
            mark_attention(app, &seat, code, &clip(&evidence)).await?;
            Ok(true)
        }
        WatchDecision::TimedOut => {
            mark_attention(
                app,
                &seat,
                ReadinessCode::Other("launch_timeout".to_owned()),
                "launch timed out",
            )
            .await?;
            Ok(true)
        }
        WatchDecision::LoginRequired { kind, evidence } => {
            mark_login(app, &seat, kind, &evidence).await?;
            Ok(true)
        }
    }
}

async fn mark_login(
    app: &App,
    seat: &Seat,
    kind: crewmux_types::LoginKind,
    evidence: &str,
) -> Result<(), Error> {
    let mut updated = seat.clone();
    LifecycleService::mark_login_required(&mut updated, kind, evidence.to_owned());
    let saved = updated;
    write(&app.store, move |conn| SeatRepo::upsert(conn, &saved)).await?;
    Ok(())
}

async fn mark_ready(app: &App, seat: &Seat, events: &mut Vec<Value>) -> Result<(), Error> {
    let id = seat.id;
    let stored = write(&app.store, move |conn| {
        let current = match SeatRepo::get(conn, &id) {
            Ok(seat) => seat,
            Err(crewmux_core::Error::SeatNotFound(_)) => return Ok(None),
            Err(err) => return Err(err),
        };
        if !matches!(current.state, SeatState::Launching | SeatState::Planned) {
            return Ok(None);
        }
        let mut updated = current;
        let event = LifecycleService::mark_ready(&mut updated, now_ts());
        let name = updated.name.as_str().to_owned();
        SeatRepo::upsert(conn, &updated)?;
        crewmux_core::EventLog::append(conn, &event)?;
        Ok(Some((event, name)))
    })
    .await?;
    if let Some((event, name)) = stored {
        events.push(event_value(&event, Some(&name)));
    }
    Ok(())
}

async fn mark_attention(
    app: &App,
    seat: &Seat,
    code: ReadinessCode,
    reason: &str,
) -> Result<(), Error> {
    let mut updated = seat.clone();
    LifecycleService::mark_attention(&mut updated, code, Some(reason.to_owned()));
    let saved = updated;
    write(&app.store, move |conn| SeatRepo::upsert(conn, &saved)).await?;
    Ok(())
}

fn clip(text: &str) -> String {
    text.chars().take(400).collect()
}
