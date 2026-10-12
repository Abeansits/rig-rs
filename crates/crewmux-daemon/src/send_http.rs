//! `POST /api/send` and `POST /api/broadcast`.
//!
//! The sender is the seat proved by `X-Rig-Seat-Id` and `X-Rig-Seat-Token`
//! (PE-011). A body `from` is not an actor: serde drops it. No seat headers
//! records `operator@host`. A wrong or partial token is refused.

use std::str::FromStr;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use crewmux_core::SeatRepo;
use crewmux_messaging::chat::{SeatProof, resolve_sender};
use crewmux_store::{Db, seat_token};
use crewmux_types::{SEAT_ID_HEADER, SEAT_TOKEN_HEADER, SeatId};
use serde::Deserialize;

use crate::deliver::{Outbound, deliver};
use crate::error::Error;
use crate::query::address_of;
use crate::state::{App, join_err};

/// Unauthenticated send. Not a seat name.
const UNAUTHENTICATED: &str = "operator@host";

pub(crate) async fn send_route(
    State(app): State<App>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    app.touch();
    route(&app, &headers, &body, false).await
}

pub(crate) async fn broadcast_route(
    State(app): State<App>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    app.touch();
    route(&app, &headers, &body, true).await
}

async fn route(app: &App, headers: &HeaderMap, body: &[u8], all: bool) -> Response {
    let parsed: SendBody = match serde_json::from_slice(body) {
        Ok(parsed) => parsed,
        Err(err) => return crate::http::fail(Error::Message(err.to_string())),
    };
    let from = match attributed(app, headers).await {
        Ok(from) => from,
        Err(err) => return crate::http::fail(err),
    };
    let targets = parsed
        .to
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
        .collect();
    let outbound = Outbound {
        from,
        targets,
        rig: parsed.rig,
        all: all || parsed.all,
        body: parsed.body,
        interact: parsed.dangerously_interact,
        reason: parsed.reason,
        verify: parsed.verify,
        track_direct: true,
    };
    match deliver(app, outbound).await {
        Ok(value) => crate::http::ok(value),
        Err(err) => crate::http::fail(err),
    }
}

async fn attributed(app: &App, headers: &HeaderMap) -> Result<String, Error> {
    match presented(headers) {
        Presented::None => Ok(UNAUTHENTICATED.to_owned()),
        Presented::Partial => Err(Error::Message("unknown sender".to_owned())),
        Presented::Pair { seat, token } => seat_sender(app, &seat, &token).await,
    }
}

async fn seat_sender(app: &App, seat: &str, token: &str) -> Result<String, Error> {
    let seat_id =
        SeatId::from_str(seat).map_err(|_| Error::Message("unknown sender".to_owned()))?;
    let store = Arc::clone(&app.store);
    let loaded = tokio::task::spawn_blocking(move || {
        store.read(move |conn| {
            let found = match SeatRepo::get(&Db::new(conn), &seat_id) {
                Ok(seat) => seat,
                Err(crewmux_core::Error::SeatNotFound(_)) => return Ok(None),
                Err(err) => return Err(crewmux_store::Error::from(err)),
            };
            let stored = seat_token(conn, &seat_id)?;
            Ok(Some((found, stored)))
        })
    })
    .await
    .map_err(join_err)?
    .map_err(Error::from)?;
    let Some((found, stored)) = loaded else {
        return Err(Error::Message("unknown sender".to_owned()));
    };
    let id = found.id.to_string();
    let name = found.name.as_str().to_owned();
    let proof = SeatProof {
        seat_id: &id,
        token,
        stored_token: stored.as_deref(),
        name: Some(name.as_str()),
    };
    resolve_sender(Some(proof), None).map_err(|err| Error::Message(err.to_string()))?;
    address_of(app, &found).await
}

fn presented(headers: &HeaderMap) -> Presented {
    let id = header_str(headers, SEAT_ID_HEADER);
    let token = header_str(headers, SEAT_TOKEN_HEADER);
    match (id, token) {
        (None, None) => Presented::None,
        (Some(id), Some(token)) if !id.is_empty() && !token.is_empty() => Presented::Pair {
            seat: id.to_owned(),
            token: token.to_owned(),
        },
        _ => Presented::Partial,
    }
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

enum Presented {
    None,
    Pair { seat: String, token: String },
    Partial,
}

#[derive(Deserialize)]
struct SendBody {
    #[serde(default)]
    to: Option<String>,
    #[serde(default)]
    rig: Option<String>,
    #[serde(default)]
    all: bool,
    body: String,
    #[serde(default)]
    dangerously_interact: bool,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    verify: bool,
}
