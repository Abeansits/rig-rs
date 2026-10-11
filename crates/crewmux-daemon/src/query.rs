//! `rig ps`, `rig capture`, `rig attach`, and `rig down`.

use crewmux_core::{LifecycleService, RigRepo, Seat, SeatRepo, SeatState};
use crewmux_mux::{CaptureRange, MuxError};
use crewmux_types::RigName;
use serde_json::{Value, json};

use crate::db::{read, write};
use crate::error::Error;
use crate::launch::now_ts;
use crate::state::App;
use crate::view::{event_value, last_lines, line_count, seat_value};

pub(crate) async fn list_seats(app: &App) -> Result<Value, Error> {
    let rigs = read(&app.store, |conn| conn.list()).await?;
    let mut body = Vec::new();
    for rig in rigs {
        let id = rig.id;
        let seats = read(&app.store, move |conn| conn.list_by_rig(&id)).await?;
        let mut rendered = Vec::new();
        for seat in seats {
            let attached = attached(app, &seat).await?;
            rendered.push(seat_value(&seat, Some(attached)));
        }
        body.push(json!({"name": rig.name.as_str(), "seats": rendered}));
    }
    Ok(json!({"rigs": body}))
}

pub(crate) async fn capture(app: &App, name: &str, lines: Option<u32>) -> Result<Value, Error> {
    let seat = find_seat(app, name).await?;
    let Some(session) = seat.tmux_session else {
        return Err(Error::Message(format!("seat {name} has no session")));
    };
    let pane = crewmux_mux::PaneRef {
        session,
        window: Some(0),
        pane: Some(0),
        pane_id: seat.pane_id.clone(),
    };
    let shot = app.mux.capture(&pane, CaptureRange::Visible).await?;
    let text = match lines {
        Some(limit) => last_lines(&shot.text, limit),
        None => shot.text,
    };
    let count = line_count(&text);
    Ok(json!({"seat": name, "text": text, "lines": count}))
}

pub(crate) async fn attach_argv(app: &App, name: &str) -> Result<Vec<String>, Error> {
    let seat = find_seat(app, name).await?;
    let Some(session) = seat.tmux_session else {
        return Err(Error::Message(format!("seat {name} has no session")));
    };
    let (binary, socket_path) = app.mux.attach_program();
    Ok(tmux_attach(&binary, &socket_path, &session))
}

/// `rig attach` execs the daemon's tmux binary against the server socket path.
///
/// `-S` ignores the caller's `TMUX_TMPDIR`. `-L` would follow that variable and
/// miss the server the daemon created.
pub(crate) fn tmux_attach(binary: &str, socket_path: &str, session: &str) -> Vec<String> {
    let name: String = session
        .chars()
        .map(|ch| if ch == '.' || ch == ':' { '_' } else { ch })
        .collect();
    vec![
        binary.to_owned(),
        "-S".to_owned(),
        socket_path.to_owned(),
        "attach".to_owned(),
        "-t".to_owned(),
        format!("={name}"),
    ]
}

pub(crate) async fn down(app: &App, rig_name: Option<&str>) -> Result<Value, Error> {
    let rigs = read(&app.store, |conn| conn.list()).await?;
    let mut killed = 0u32;
    let mut events = Vec::new();
    for rig in rigs {
        if let Some(wanted) = rig_name
            && rig.name.as_str() != wanted
        {
            continue;
        }
        let id = rig.id;
        let seats = read(&app.store, move |conn| conn.list_by_rig(&id)).await?;
        for seat in seats {
            if stop_seat(app, &seat, &mut events).await? {
                killed = killed.saturating_add(1);
            }
        }
    }
    if let Some(wanted) = rig_name
        && events.is_empty()
        && killed == 0
        && !rig_exists(app, wanted).await?
    {
        return Err(Error::Message(format!("rig not found: {wanted}")));
    }
    Ok(json!({"stopped": true, "sessions_killed": killed, "events": events}))
}

async fn rig_exists(app: &App, name: &str) -> Result<bool, Error> {
    let name = RigName::new(name).map_err(|err| Error::Message(err.to_string()))?;
    let found = read(&app.store, move |conn| conn.get_by_name(&name)).await?;
    Ok(found.is_some())
}

async fn stop_seat(app: &App, seat: &Seat, events: &mut Vec<Value>) -> Result<bool, Error> {
    let id = seat.id;
    let prepared = write(&app.store, move |conn| {
        let current = match SeatRepo::get(conn, &id) {
            Ok(seat) => seat,
            Err(crewmux_core::Error::SeatNotFound(_)) => return Ok(None),
            Err(err) => return Err(err),
        };
        if current.state == SeatState::Stopped {
            return Ok(None);
        }
        let session = current.tmux_session.clone();
        let mut updated = current;
        let event = LifecycleService::mark_stopped(&mut updated, now_ts());
        let name = updated.name.as_str().to_owned();
        SeatRepo::upsert(conn, &updated)?;
        crewmux_core::EventLog::append(conn, &event)?;
        Ok(Some((event, name, session)))
    })
    .await?;
    let Some((event, name, session)) = prepared else {
        return Ok(false);
    };
    let mut killed = false;
    if let Some(session) = session {
        match app.mux.kill_session(&session).await {
            Ok(()) => killed = true,
            Err(MuxError::NotFound(_)) => {}
            Err(err) => return Err(err.into()),
        }
    }
    events.push(event_value(&event, Some(&name)));
    Ok(killed)
}

async fn attached(app: &App, seat: &Seat) -> Result<bool, Error> {
    let Some(session) = &seat.tmux_session else {
        return Ok(false);
    };
    let clients = app.mux.human_clients(session).await?;
    Ok(!clients.is_empty())
}

async fn find_seat(app: &App, name: &str) -> Result<Seat, Error> {
    let seats = read(&app.store, |conn| conn.list_all()).await?;
    let mut live = Vec::new();
    let mut resting = Vec::new();
    for seat in seats {
        if seat.name.as_str() != name {
            continue;
        }
        if matches!(seat.state, SeatState::Stopped | SeatState::Dead) {
            resting.push(seat);
        } else {
            live.push(seat);
        }
    }
    if live.len() > 1 {
        return Err(Error::Message(format!("seat {name} is ambiguous")));
    }
    if let Some(seat) = live.pop() {
        return Ok(seat);
    }
    if resting.len() > 1 {
        return Err(Error::Message(format!("seat {name} is ambiguous")));
    }
    resting
        .pop()
        .ok_or_else(|| Error::Message(format!("seat not found: {name}")))
}
