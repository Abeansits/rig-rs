//! `GET /api/events` on the slice 1 daemon.
//!
//! The log is [`crewmux_core::EventLog`]. Seat names (not ULIDs) are the TUI
//! row keys. A cursor inside the log is replayed. A cursor ahead of the log
//! gets one `gap`, then live events whose ids stay above that cursor.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::time::Duration;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use crewmux_api::{SeatKey, StreamEvent, StreamKind, StreamPayload};
use crewmux_core::{DomainEvent, EventLog, SeatRepo, SeatState, Seq};
use crewmux_store::Db;
use serde::Deserialize;
use serde_json::json;

use crate::db::read;
use crate::error::Error;
use crate::state::App;

/// Route the TUI opens for the live tail.
pub(crate) const PATH: &str = "/api/events";

const BATCH: usize = 32;
const POLL: Duration = Duration::from_millis(200);

#[derive(Debug, Default, Deserialize)]
pub(crate) struct Resume {
    #[serde(default)]
    last_event_id: Option<String>,
}

struct Follow {
    seq: u64,
    offset: u64,
    gap_id: Option<u64>,
}

struct Projected {
    kind: StreamKind,
    payload: StreamPayload,
}

struct Tail {
    app: App,
    follow: Follow,
    pending: VecDeque<StreamEvent>,
    primed: bool,
    again: bool,
}

pub(crate) async fn events(
    State(app): State<App>,
    headers: HeaderMap,
    Query(resume): Query<Resume>,
) -> Response {
    let tip = match tip_of(&app).await {
        Ok(tip) => tip,
        Err(err) => return fail(&err),
    };
    let follow = follow(resume_cursor(&headers, &resume), tip);
    let mut pending = VecDeque::new();
    if let Some(id) = follow.gap_id {
        pending.push_back(gap(id));
    }
    let tail = Tail {
        app,
        follow,
        pending,
        primed: false,
        again: false,
    };
    Sse::new(futures_util::stream::unfold(tail, step))
        .keep_alive(KeepAlive::default())
        .into_response()
}

fn follow(last: Option<u64>, tip: u64) -> Follow {
    match last {
        None => Follow {
            seq: tip,
            offset: 0,
            gap_id: None,
        },
        Some(id) if id <= tip => Follow {
            seq: id,
            offset: 0,
            gap_id: None,
        },
        Some(id) => Follow {
            seq: tip,
            offset: id.saturating_add(1).saturating_sub(tip),
            gap_id: Some(id.saturating_add(1)),
        },
    }
}

fn resume_cursor(headers: &HeaderMap, resume: &Resume) -> Option<u64> {
    let header = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok());
    header.or(resume.last_event_id.as_deref()).and_then(|text| {
        if text.len() > 20 {
            None
        } else {
            text.parse().ok()
        }
    })
}

fn project(event: &DomainEvent, seat: Option<&SeatKey>) -> Option<Projected> {
    let seat = seat?;
    let (kind, state) = match event {
        DomainEvent::RigCreated { .. } => return None,
        DomainEvent::SeatLaunched { .. } => {
            (StreamKind::SeatStateChanged, Some(SeatState::Launching))
        }
        DomainEvent::SeatReady { .. } => (StreamKind::SeatStateChanged, Some(SeatState::Ready)),
        DomainEvent::SeatStopped { .. } => (StreamKind::SeatStateChanged, Some(SeatState::Stopped)),
        DomainEvent::SeatDied { .. } => (StreamKind::SeatDied, None),
    };
    Some(Projected {
        kind,
        payload: StreamPayload {
            seat: seat.clone(),
            state,
            summary: None,
        },
    })
}

fn note_seq(
    follow: &mut Follow,
    pending: &mut VecDeque<StreamEvent>,
    seq: u64,
    projected: Option<Projected>,
) {
    if seq <= follow.seq {
        return;
    }
    if seq > follow.seq.saturating_add(1) {
        let missed = seq.saturating_sub(1);
        pending.push_back(gap(missed.saturating_add(follow.offset)));
    }
    follow.seq = seq;
    if let Some(projected) = projected {
        pending.push_back(StreamEvent {
            id: seq.saturating_add(follow.offset),
            kind: projected.kind,
            payload: Some(projected.payload),
        });
    }
}

fn enqueue(tail: &mut Tail, rows: Vec<(u64, DomainEvent, Option<SeatKey>)>) {
    for (seq, event, seat) in rows {
        let projected = project(&event, seat.as_ref());
        note_seq(&mut tail.follow, &mut tail.pending, seq, projected);
    }
}

async fn step(mut tail: Tail) -> Option<(Result<Event, Infallible>, Tail)> {
    loop {
        if let Some(event) = tail.pending.pop_front() {
            return Some((Ok(frame(&event)), tail));
        }
        if !wait_ready(&mut tail).await {
            return None;
        }
        let rows = pull(&tail.app, tail.follow.seq).await.ok()?;
        tail.again = rows.len() == BATCH;
        enqueue(&mut tail, rows);
    }
}

async fn wait_ready(tail: &mut Tail) -> bool {
    if tail.app.cancel.is_cancelled() {
        return false;
    }
    if tail.again || !tail.primed {
        tail.primed = true;
        return true;
    }
    tokio::select! {
        biased;
        () = tail.app.cancel.cancelled() => false,
        () = tokio::time::sleep(POLL) => true,
    }
}

async fn tip_of(app: &App) -> Result<u64, Error> {
    read(&app.store, |conn| Ok(EventLog::tip(conn)?.0)).await
}

async fn pull(app: &App, after: u64) -> Result<Vec<(u64, DomainEvent, Option<SeatKey>)>, Error> {
    read(&app.store, move |conn| load_after(conn, after)).await
}

fn load_after(
    conn: &Db<'_>,
    after: u64,
) -> Result<Vec<(u64, DomainEvent, Option<SeatKey>)>, crewmux_core::Error> {
    let rows = EventLog::after(conn, Seq(after))?;
    let mut out = Vec::with_capacity(rows.len());
    for (seq, event) in rows {
        let name = name_of(conn, &event)?;
        out.push((seq.0, event, name));
    }
    Ok(out)
}

fn name_of(conn: &Db<'_>, event: &DomainEvent) -> Result<Option<SeatKey>, crewmux_core::Error> {
    let Some(id) = seat_id(event) else {
        return Ok(None);
    };
    match SeatRepo::get(conn, id) {
        Ok(seat) => Ok(SeatKey::parse(seat.name.as_str()).ok()),
        Err(crewmux_core::Error::SeatNotFound(_)) => Ok(None),
        Err(err) => Err(err),
    }
}

fn seat_id(event: &DomainEvent) -> Option<&crewmux_types::SeatId> {
    match event {
        DomainEvent::RigCreated { .. } => None,
        DomainEvent::SeatLaunched { seat, .. }
        | DomainEvent::SeatReady { seat, .. }
        | DomainEvent::SeatStopped { seat, .. }
        | DomainEvent::SeatDied { seat, .. } => Some(seat),
    }
}

fn gap(id: u64) -> StreamEvent {
    StreamEvent {
        id,
        kind: StreamKind::Gap,
        payload: None,
    }
}

fn frame(event: &StreamEvent) -> Event {
    let data = serde_json::to_string(event).unwrap_or_default();
    Event::default().id(event.id.to_string()).data(data)
}

fn fail(err: &Error) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": err.to_string()})),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use crewmux_api::{SeatKey, StreamKind};
    use crewmux_core::{DomainEvent, SeatState, Timestamp};
    use crewmux_types::{SeatId, SessionId};

    use super::{follow, note_seq, project};

    fn seat() -> SeatKey {
        SeatKey::parse("alpha").expect("seat")
    }

    #[test]
    fn fresh_cursor_starts_at_the_tip() {
        let cursor = follow(None, 4);
        assert_eq!(cursor.seq, 4);
        assert_eq!(cursor.offset, 0);
        assert!(cursor.gap_id.is_none());
    }

    #[test]
    fn resume_inside_the_log_replays_after_that_id() {
        let cursor = follow(Some(2), 4);
        assert_eq!(cursor.seq, 2);
        assert_eq!(cursor.offset, 0);
        assert!(cursor.gap_id.is_none());
    }

    #[test]
    fn resume_ahead_of_the_log_is_one_gap() {
        let cursor = follow(Some(10), 8);
        assert_eq!(cursor.seq, 8);
        assert_eq!(cursor.gap_id, Some(11));
        assert_eq!(cursor.offset, 3);
        assert!(9_u64.saturating_add(cursor.offset) > 11);
    }

    #[test]
    fn domain_kinds_map_to_stream_kinds() {
        let key = seat();
        let id = SeatId::new();
        let at = Timestamp::from_millis(0);
        let launched = DomainEvent::SeatLaunched {
            seat: id,
            session: SessionId::new(),
            at,
        };
        let projected = project(&launched, Some(&key)).expect("launched");
        assert_eq!(projected.kind, StreamKind::SeatStateChanged);
        assert_eq!(projected.payload.state, Some(SeatState::Launching));
        assert_eq!(projected.payload.seat.as_str(), "alpha");

        let died = DomainEvent::SeatDied {
            seat: SeatId::new(),
            at,
        };
        let projected = project(&died, Some(&key)).expect("died");
        assert_eq!(projected.kind, StreamKind::SeatDied);
        assert!(projected.payload.state.is_none());

        let created = DomainEvent::RigCreated {
            rig: crewmux_types::RigId::new(),
            name: crewmux_types::RigName::new("pair").expect("rig"),
            at,
        };
        assert!(project(&created, Some(&key)).is_none());
        assert!(project(&died, None).is_none());
    }

    #[test]
    fn a_hole_emits_one_gap_then_the_event() {
        let mut cursor = follow(Some(5), 8);
        let mut pending = VecDeque::new();
        let died = DomainEvent::SeatDied {
            seat: SeatId::new(),
            at: Timestamp::from_millis(0),
        };
        let projected = project(&died, Some(&seat()));
        note_seq(&mut cursor, &mut pending, 8, projected);
        let gap = pending.front().expect("gap");
        assert_eq!(gap.kind, StreamKind::Gap);
        assert_eq!(gap.id, 7);
        let event = pending.get(1).expect("event");
        assert_eq!(event.kind, StreamKind::SeatDied);
        assert_eq!(event.id, 8);
        assert_eq!(cursor.seq, 8);
    }
}
