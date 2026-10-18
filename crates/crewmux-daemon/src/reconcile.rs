//! At startup, make the store match the multiplexer.
//!
//! A daemon killed mid-up leaves seats in `planned` or `launching` with stubs
//! still alive. Those rows used to count as already running, so the next `up`
//! refused to start. Incomplete seats are stopped in tmux and marked dead.
//! A ready or attention seat whose session is gone is marked dead as well.

use crewmux_core::{SeatRepo, SeatState};
use crewmux_mux::MuxError;

use crate::death::mark_dead;
use crate::error::Error;
use crate::state::App;

use crate::db::read;

/// Drop sessions the store no longer treats as a finished launch, and mark
/// seats dead when tmux no longer has them.
///
/// # Errors
///
/// Returns [`Error`] when the store or the multiplexer fails.
pub(crate) async fn reconcile(app: &App) -> Result<(), Error> {
    let seats = read(&app.store, |conn| conn.list_all()).await?;
    for seat in seats {
        if matches!(seat.state, SeatState::Stopped | SeatState::Dead) {
            continue;
        }
        let alive = match &seat.tmux_session {
            Some(name) => app.mux.has_session(name).await?,
            None => false,
        };
        let incomplete = matches!(seat.state, SeatState::Planned | SeatState::Launching);
        if alive
            && incomplete
            && let Some(name) = &seat.tmux_session
        {
            match app.mux.kill_session(name).await {
                Ok(()) | Err(MuxError::NotFound(_)) => {}
                Err(err) => return Err(err.into()),
            }
        }
        if incomplete || !alive {
            let capture = crate::death::last_capture(app, &seat).await;
            mark_dead(app, &seat, None, capture).await?;
        }
    }
    Ok(())
}
