//! `POST /api/send` and `POST /api/broadcast`.

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use serde::Deserialize;

use crate::deliver::{Outbound, deliver};
use crate::error::Error;
use crate::state::App;

pub(crate) async fn send_route(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    route(&app, &body, false).await
}

pub(crate) async fn broadcast_route(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    route(&app, &body, true).await
}

async fn route(app: &App, body: &[u8], all: bool) -> Response {
    let parsed: SendBody = match serde_json::from_slice(body) {
        Ok(parsed) => parsed,
        Err(err) => return crate::http::fail(Error::Message(err.to_string())),
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
        from: parsed.from.unwrap_or_else(|| "operator@host".to_owned()),
        targets,
        rig: parsed.rig,
        all: all || parsed.all,
        body: parsed.body,
        interact: parsed.dangerously_interact,
        reason: parsed.reason,
        verify: parsed.verify,
    };
    match deliver(app, outbound).await {
        Ok(value) => crate::http::ok(value),
        Err(err) => crate::http::fail(err),
    }
}

#[derive(Deserialize)]
struct SendBody {
    #[serde(default)]
    from: Option<String>,
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
