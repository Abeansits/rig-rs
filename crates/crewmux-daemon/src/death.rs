//! Mark seats dead when the multiplexer reports a pane or window death.
//!
//! The clear path sends Ctrl-U only. Claude does not get Escape, Ctrl-C,
//! Ctrl-D, or Ctrl-G, and a compact timeout does not kill the pane. `seat_died`
//! still records the exit status and the last capture so the next unexplained
//! exit can be read from the event.

use crewmux_core::{LifecycleService, Seat, SeatRepo, SeatState};
use crewmux_mux::{CaptureRange, MuxEvent};

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
        MuxEvent::PaneDied { pane_id, status } => {
            if let Some(seat) = read(&app.store, move |conn| conn.get_by_pane(&pane_id)).await? {
                let capture = last_capture(app, &seat).await;
                mark_dead(app, &seat, status, capture).await?;
            }
        }
        MuxEvent::WindowClosed { session, .. } => {
            if !session.is_empty() {
                let seats = read(&app.store, |conn| conn.list_all()).await?;
                for seat in seats {
                    if seat.tmux_session.as_deref() == Some(session.as_str()) {
                        let capture = last_capture(app, &seat).await;
                        mark_dead(app, &seat, None, capture).await?;
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

pub(crate) async fn mark_dead(
    app: &App,
    seat: &Seat,
    exit_status: Option<i32>,
    capture: Option<String>,
) -> Result<(), Error> {
    let id = seat.id;
    let logged = capture.clone();
    tracing::info!(
        seat = %id,
        exit_status = ?exit_status,
        capture = logged.as_deref().unwrap_or(""),
        "seat_died"
    );
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
        let event = LifecycleService::mark_dead(&mut updated, now_ts(), exit_status, capture);
        SeatRepo::upsert(conn, &updated)?;
        crewmux_core::EventLog::append(conn, &event)?;
        Ok(())
    })
    .await?;
    Ok(())
}

pub(crate) async fn last_capture(app: &App, seat: &Seat) -> Option<String> {
    let pane = crate::query::pane_of(seat).ok()?;
    let shot = app.mux.capture(&pane, CaptureRange::Visible).await.ok()?;
    Some(bound_capture(shot.text))
}

fn bound_capture(text: String) -> String {
    const LIMIT: usize = 8_000;
    if text.len() <= LIMIT {
        return text;
    }
    let mut start = text.len().saturating_sub(LIMIT);
    while start < text.len() && !text.is_char_boundary(start) {
        start = start.saturating_add(1);
    }
    text.get(start..).unwrap_or("").to_owned()
}
