//! Typed helpers over the blocking client.
//!
//! `GET /api/seats` is slice 1's list: rigs, then seats with `name`, `runtime`,
//! `state`, and `tmux_session`. The TUI folds that into a [`Snapshot`].
//! `POST /api/seats/attach` returns the argv the daemon wants executed.

use serde::Deserialize;

use crewmux_core::SeatState;
use crewmux_types::{
    LoginKind, PodName, ReadinessCode, RigName, RuntimeKind, SeatKey, TmuxSession,
};

use crate::client::Client;
use crate::endpoint::{AttachInfo, Endpoint, ListSeats};
use crate::error::Error;
use crate::model::{SeatView, Snapshot};

#[derive(Deserialize)]
struct Listed {
    rigs: Vec<ListedRig>,
}

#[derive(Deserialize)]
struct ListedRig {
    name: String,
    seats: Vec<ListedSeat>,
}

#[derive(Deserialize)]
struct ListedSeat {
    name: String,
    runtime: RuntimeKind,
    state: String,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    tmux_session: Option<String>,
}

#[derive(Deserialize)]
struct AttachBody {
    argv: Vec<String>,
}

impl Client {
    /// `GET /api/seats`, mapped into the explorer snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the daemon is unreachable or the body is not a seat list.
    pub fn hydrate(&self) -> Result<Snapshot, Error> {
        let response = self.request(ListSeats::METHOD, ListSeats::PATH, None)?;
        if !(200..300).contains(&response.status) {
            return Err(Error::Status {
                path: ListSeats::PATH.to_owned(),
                status: response.status,
            });
        }
        let listed: Listed = serde_json::from_str(&response.body).map_err(|err| Error::Json {
            path: ListSeats::PATH.to_owned(),
            message: err.to_string(),
        })?;
        Ok(snapshot_of(listed))
    }

    /// `POST /api/seats/attach`. `seat` is the seat name, not a ULID.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the seat is unknown or the body has no argv.
    pub fn attach_argv(&self, seat: &str) -> Result<Vec<String>, Error> {
        let body =
            serde_json::to_vec(&serde_json::json!({"seat": seat})).map_err(|err| Error::Json {
                path: AttachInfo::PATH.to_owned(),
                message: err.to_string(),
            })?;
        let response = self.request(AttachInfo::METHOD, AttachInfo::PATH, Some(&body))?;
        if !(200..300).contains(&response.status) {
            return Err(Error::Status {
                path: AttachInfo::PATH.to_owned(),
                status: response.status,
            });
        }
        let parsed: AttachBody =
            serde_json::from_str(&response.body).map_err(|err| Error::Json {
                path: AttachInfo::PATH.to_owned(),
                message: err.to_string(),
            })?;
        Ok(parsed.argv)
    }
}

fn snapshot_of(listed: Listed) -> Snapshot {
    let mut seats = Vec::new();
    for rig in listed.rigs {
        let Ok(rig_name) = RigName::new(rig.name.clone()) else {
            continue;
        };
        let Some(pod) = pod_of(&rig.name) else {
            continue;
        };
        for seat in rig.seats {
            let Some(view) = seat_view(&rig_name, &pod, seat) else {
                continue;
            };
            seats.push(view);
        }
    }
    Snapshot {
        seats,
        health: None,
        activity: Vec::new(),
    }
}

fn pod_of(name: &str) -> Option<PodName> {
    PodName::parse(name.to_owned())
        .ok()
        .or_else(|| PodName::parse("pod").ok())
}

fn seat_view(rig_name: &RigName, pod: &PodName, seat: ListedSeat) -> Option<SeatView> {
    let name = SeatKey::parse(seat.name).ok()?;
    let state = seat_state(&seat.state, seat.code.as_deref())?;
    let session = seat
        .tmux_session
        .and_then(|text| TmuxSession::parse(text).ok());
    Some(SeatView {
        id: name.clone(),
        name,
        rig_id: rig_name.clone(),
        rig_name: rig_name.clone(),
        pod: pod.clone(),
        state,
        runtime: seat.runtime,
        session,
        spec: None,
    })
}

fn seat_state(state: &str, code: Option<&str>) -> Option<SeatState> {
    match state {
        "planned" => Some(SeatState::Planned),
        "launching" => Some(SeatState::Launching),
        "ready" => Some(SeatState::Ready),
        "stopped" => Some(SeatState::Stopped),
        "dead" => Some(SeatState::Dead),
        "attention" => Some(SeatState::Attention(ReadinessCode::parse(
            code.unwrap_or("unrecognised_screen"),
        ))),
        "login_required" => Some(SeatState::LoginRequired {
            kind: LoginKind::parse(code.unwrap_or("menu")),
            evidence: String::new(),
        }),
        _ => None,
    }
}
