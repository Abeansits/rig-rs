//! Mark seats dead when the multiplexer reports a pane or window death.

use crewmux_core::{LifecycleService, Seat, SeatRepo, SeatState};
use crewmux_mux::MuxEvent;

use crate::db::{read, write};
use crate::error::Error;
use crate::launch::now_ts;
use crate::state::App;

pub(crate) async fn supervise(app: App) {
    let mut deaths = app.mux.death_events();
    loop {
        tokio::select! {
            () = app.cancel.cancelled() => break,
            event = deaths.recv() => {
                let Some(event) = event else { break };
                if let Err(err) = apply(&app, event).await {
                    tracing::warn!(error = %err, "seat death");
                }
            }
        }
    }
}

async fn apply(app: &App, event: MuxEvent) -> Result<(), Error> {
    match event {
        MuxEvent::PaneDied { pane_id, .. } => {
            if let Some(seat) = read(&app.store, move |conn| conn.get_by_pane(&pane_id)).await? {
                mark_dead(app, &seat).await?;
            }
        }
        MuxEvent::WindowClosed { session, .. } => {
            if !session.is_empty() {
                let seats = read(&app.store, |conn| conn.list_all()).await?;
                for seat in seats {
                    if seat.tmux_session.as_deref() == Some(session.as_str()) {
                        mark_dead(app, &seat).await?;
                    }
                }
            }
        }
        MuxEvent::Output { .. }
        | MuxEvent::SessionsChanged
        | MuxEvent::ServerExited
        | MuxEvent::Reconnected
        | MuxEvent::Gap { .. } => {}
    }
    Ok(())
}

async fn mark_dead(app: &App, seat: &Seat) -> Result<(), Error> {
    if matches!(seat.state, SeatState::Dead | SeatState::Stopped) {
        return Ok(());
    }
    let mut updated = seat.clone();
    let event = LifecycleService::mark_dead(&mut updated, now_ts());
    let saved = updated;
    write(&app.store, move |conn| {
        SeatRepo::upsert(conn, &saved)?;
        crewmux_core::EventLog::append(conn, &event)?;
        Ok(())
    })
    .await?;
    Ok(())
}
