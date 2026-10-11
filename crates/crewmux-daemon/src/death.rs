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

pub(crate) async fn mark_dead(app: &App, seat: &Seat) -> Result<(), Error> {
    let id = seat.id;
    write(&app.store, move |conn| {
        let current = match SeatRepo::get(conn, &id) {
            Ok(seat) => seat,
            Err(crewmux_core::Error::SeatNotFound(_)) => return Ok(()),
            Err(err) => return Err(err),
        };
        if matches!(current.state, SeatState::Dead | SeatState::Stopped) {
            return Ok(());
        }
        let mut updated = current;
        let event = LifecycleService::mark_dead(&mut updated, now_ts());
        SeatRepo::upsert(conn, &updated)?;
        crewmux_core::EventLog::append(conn, &event)?;
        Ok(())
    })
    .await?;
    Ok(())
}
