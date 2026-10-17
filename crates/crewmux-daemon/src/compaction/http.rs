//! Compaction HTTP routes.

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::post;
use crewmux_api::{CompactCancel, CompactStatus, CompactTrigger, Endpoint};

use crate::error::Error;
use crate::http::{fail, ok};
use crate::state::App;

use super::apply;
use super::trigger;

pub(crate) fn routes() -> Router<App> {
    Router::new()
        .route(CompactTrigger::PATH, post(trigger_route))
        .route(CompactCancel::PATH, post(cancel_route))
        .route(CompactStatus::PATH, post(status_route))
        .route("/api/seats/{key}/hooks", post(hook_route))
}

async fn trigger_route(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    match trigger::trigger(&app, &body).await {
        Ok(value) => ok(value),
        Err(err) => fail(err),
    }
}

async fn cancel_route(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    match trigger::cancel(&app, &body).await {
        Ok(value) => ok(value),
        Err(err) => fail(err),
    }
}

async fn status_route(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    match trigger::status(&app, &body).await {
        Ok(value) => ok(value),
        Err(err) => fail(err),
    }
}

async fn hook_route(State(app): State<App>, Path(key): Path<String>, body: Bytes) -> Response {
    app.touch();
    match apply::ingest(&app, &key, &body).await {
        Ok(value) => ok(value),
        Err(err) => fail(Error::Message(err.to_string())),
    }
}
