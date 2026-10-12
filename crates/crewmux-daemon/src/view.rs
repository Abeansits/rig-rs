//! JSON shapes for `rig up`, `rig ps`, `rig capture`, and `rig down`.

use crewmux_core::{DomainEvent, Seat};
use serde_json::{Value, json};

pub(crate) fn seat_value(seat: &Seat, attached: Option<bool>) -> Value {
    let mut value = json!({
        "name": seat.name.as_str(),
        "runtime": seat.runtime.as_str(),
        "state": seat.state.as_str(),
    });
    if let Some(code) = seat.state.code() {
        insert(&mut value, "code", json!(code.as_str()));
    }
    if let Some(kind) = seat.state.login_kind() {
        insert(&mut value, "login_kind", json!(kind.as_str()));
    }
    if let Some(reason) = &seat.reason {
        insert(&mut value, "reason", json!(reason));
    }
    if let Some(session) = &seat.tmux_session {
        insert(&mut value, "tmux_session", json!(session));
    }
    if let Some(attached) = attached {
        insert(&mut value, "attached", json!(attached));
    }
    value
}

pub(crate) fn event_value(event: &DomainEvent, seat: Option<&str>) -> Value {
    match seat {
        Some(name) => json!({"kind": event.kind(), "seat": name}),
        None => json!({"kind": event.kind()}),
    }
}

pub(crate) fn status_of(seats: &[Seat]) -> &'static str {
    let launch_failed = seats.iter().any(|seat| {
        seat.tmux_session.is_none() && matches!(seat.state, crewmux_core::SeatState::Attention(_))
    });
    if launch_failed {
        return "partial";
    }
    if seats.iter().any(|seat| {
        matches!(
            seat.state,
            crewmux_core::SeatState::Attention(_) | crewmux_core::SeatState::LoginRequired { .. }
        )
    }) {
        return "attention";
    }
    if seats
        .iter()
        .any(|seat| matches!(seat.state, crewmux_core::SeatState::Dead))
    {
        return "dead";
    }
    "ready"
}

fn insert(value: &mut Value, key: &str, item: Value) {
    if let Some(object) = value.as_object_mut() {
        object.insert(key.to_owned(), item);
    }
}

pub(crate) fn last_lines(text: &str, lines: u32) -> String {
    let limit = usize::try_from(lines).unwrap_or(usize::MAX);
    let all: Vec<&str> = text.lines().collect();
    let skip = all.len().saturating_sub(limit);
    all.into_iter().skip(skip).collect::<Vec<_>>().join("\n")
}

pub(crate) fn line_count(text: &str) -> u32 {
    u32::try_from(text.lines().count()).unwrap_or(u32::MAX)
}
