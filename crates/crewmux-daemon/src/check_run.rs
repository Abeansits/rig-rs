//! Restore-check over the seats this daemon can see.

use crewmux_core::{SeatRepo, SeatState};
use crewmux_durability::{SeatSignal, restore_check};
use serde_json::Value;

use crate::db::read;
use crate::error::Error;
use crate::state::App;

/// Read-only readiness report.
///
/// # Errors
///
/// Returns [`Error`] when the seat list cannot be read.
pub(crate) async fn report(app: &App) -> Result<Value, Error> {
    let seats = read(&app.store, |conn| conn.list_all()).await?;
    let mut named = Vec::new();
    for seat in &seats {
        let resume = has_resume(app, seat).await?;
        named.push((
            seat.name.as_str().to_owned(),
            signal_of(&seat.state, resume),
        ));
    }
    let pairs: Vec<(&str, SeatSignal)> = named
        .iter()
        .map(|(name, signal)| (name.as_str(), *signal))
        .collect();
    let report = restore_check(true, &pairs);
    serde_json::to_value(report).map_err(|err| Error::Message(err.to_string()))
}

fn signal_of(state: &SeatState, resume: bool) -> SeatSignal {
    match state {
        SeatState::Ready => SeatSignal::Up { resume },
        SeatState::Dead | SeatState::Stopped => SeatSignal::Down,
        SeatState::Attention(_) | SeatState::LoginRequired { .. } => SeatSignal::Attention,
        SeatState::Planned | SeatState::Launching => SeatSignal::Unknown,
    }
}

async fn has_resume(app: &App, seat: &crewmux_core::Seat) -> Result<bool, Error> {
    let id = seat.id.to_string();
    let store = std::sync::Arc::clone(&app.store);
    let row = tokio::task::spawn_blocking(move || {
        store.read(move |conn| crewmux_store::load_seat_restore(conn, &id))
    })
    .await
    .map_err(crate::state::join_err)??;
    Ok(row.is_some_and(|row| row.resume_value.is_some()))
}
