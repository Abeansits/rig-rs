// Derived from openrig packages/daemon/src/routes/context-packs.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! `/api/context-packs` routes. Library work runs on a blocking thread.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use crewmux_orchestration::context_packs::{Library, PackError};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::context_address::resolve_address_route;
use crate::context_deliver::deliver_pack;
use crate::context_ops::{
    compose_route, library_list, pieces_route, preview_route, remove_route, show_route, sync_route,
};
use crate::context_profile::profile_route;
use crate::context_recap::recap_route;
use crate::state::App;

/// A route failure that already has an HTTP status.
pub(crate) enum RouteError {
    /// Library error. Status comes from [`PackError::code`](crewmux_orchestration::context_packs::PackCode).
    Pack(PackError),
    /// A named status and JSON body.
    Http { status: u16, body: Value },
}

impl From<PackError> for RouteError {
    fn from(err: PackError) -> Self {
        Self::Pack(err)
    }
}

impl RouteError {
    pub(crate) fn named(status: u16, error: &str, message: impl Into<String>) -> Self {
        Self::Http {
            status,
            body: json!({"error": error, "message": message.into()}),
        }
    }
}

pub(crate) fn routes() -> Router<App> {
    Router::new()
        .route("/api/context-packs/library", get(library))
        .route("/api/context-packs/library/sync", post(sync))
        .route("/api/context-packs/library/compose", post(compose))
        .route(
            "/api/context-packs/library/by-ref",
            get(by_ref).delete(remove),
        )
        .route("/api/context-packs/library/by-ref/preview", get(preview))
        .route("/api/context-packs/library/by-ref/pieces", get(pieces))
        .route("/api/context-packs/library/by-ref/profile", get(profile))
        .route("/api/context-packs/library/resolve-address", get(address))
        .route("/api/context-packs/deliver", post(deliver))
        .route("/api/context-packs/recap", post(recap))
}

#[derive(Debug, Deserialize)]
struct RefQuery {
    #[serde(rename = "ref")]
    refer: Option<String>,
    situation: Option<String>,
    runtime: Option<String>,
    budget: Option<String>,
    profile: Option<String>,
    rig: Option<String>,
    seat: Option<String>,
    mission: Option<String>,
    slice: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AddressQuery {
    address: Option<String>,
}

async fn library(State(app): State<App>) -> Response {
    app.touch();
    finish(run(&app, library_list).await)
}

async fn sync(State(app): State<App>) -> Response {
    app.touch();
    finish(run(&app, sync_route).await)
}

async fn compose(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let text = String::from_utf8_lossy(&body).into_owned();
    match run(&app, move |lib| compose_route(lib, &text)).await {
        Ok(value) => json_at(201, value),
        Err(response) => *response,
    }
}

async fn by_ref(State(app): State<App>, Query(query): Query<RefQuery>) -> Response {
    app.touch();
    let refer = query.refer;
    finish(run(&app, move |lib| show_route(lib, refer.as_deref())).await)
}

async fn remove(State(app): State<App>, Query(query): Query<RefQuery>) -> Response {
    app.touch();
    let refer = query.refer;
    finish(run(&app, move |lib| remove_route(lib, refer.as_deref())).await)
}

async fn preview(State(app): State<App>, Query(query): Query<RefQuery>) -> Response {
    app.touch();
    let refer = query.refer;
    finish(run(&app, move |lib| preview_route(lib, refer.as_deref())).await)
}

async fn pieces(State(app): State<App>, Query(query): Query<RefQuery>) -> Response {
    app.touch();
    let refer = query.refer;
    finish(run(&app, move |lib| pieces_route(lib, refer.as_deref())).await)
}

async fn profile(State(app): State<App>, Query(query): Query<RefQuery>) -> Response {
    app.touch();
    let grants = app.context.grants();
    finish(
        run(&app, move |lib| {
            profile_route(lib, &query_view(&query), &grants)
        })
        .await,
    )
}

async fn address(State(app): State<App>, Query(query): Query<AddressQuery>) -> Response {
    app.touch();
    let address = query.address;
    finish(
        run(&app, move |lib| {
            resolve_address_route(lib, address.as_deref())
        })
        .await,
    )
}

async fn deliver(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let owned = body.to_vec();
    let hold = Arc::clone(&app.context);
    if !hold.available() {
        return unavailable();
    }
    let prepared = tokio::task::spawn_blocking(move || {
        hold.with_lib(|lib| crate::context_deliver::prepare(lib, &owned))
    })
    .await;
    let prepared = match prepared {
        Ok(Ok(Ok(value))) => value,
        Ok(Ok(Err(err))) => return route_response(err),
        Ok(Err(err)) => return pack_response(&err),
        Err(err) => return failed_join(&err),
    };
    match deliver_pack(&app, prepared).await {
        Ok(value) => json_at(200, value),
        Err(err) => route_response(err),
    }
}

async fn recap(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let owned = body.to_vec();
    let (topology, _, _) = app.context.grants();
    let joined = tokio::task::spawn_blocking(move || recap_route(&topology, &owned)).await;
    match joined {
        Ok(Ok(value)) => json_at(200, value),
        Ok(Err(err)) => route_response(err),
        Err(err) => failed_join(&err),
    }
}

pub(crate) struct QueryView {
    pub refer: Option<String>,
    pub situation: Option<String>,
    pub runtime: Option<String>,
    pub budget: Option<String>,
    pub profile: Option<String>,
    pub rig: Option<String>,
    pub seat: Option<String>,
    pub mission: Option<String>,
    pub slice: Option<String>,
}

fn query_view(query: &RefQuery) -> QueryView {
    QueryView {
        refer: query.refer.clone(),
        situation: query.situation.clone(),
        runtime: query.runtime.clone(),
        budget: query.budget.clone(),
        profile: query.profile.clone(),
        rig: query.rig.clone(),
        seat: query.seat.clone(),
        mission: query.mission.clone(),
        slice: query.slice.clone(),
    }
}

async fn run<T: Send + 'static>(
    app: &App,
    f: impl FnOnce(&mut Library) -> Result<T, RouteError> + Send + 'static,
) -> Result<T, Box<Response>> {
    if !app.context.available() {
        return Err(Box::new(unavailable()));
    }
    let hold = Arc::clone(&app.context);
    match tokio::task::spawn_blocking(move || hold.with_lib(f)).await {
        Ok(Ok(Ok(value))) => Ok(value),
        Ok(Ok(Err(err))) => Err(Box::new(route_response(err))),
        Ok(Err(err)) => Err(Box::new(pack_response(&err))),
        Err(err) => Err(Box::new(failed_join(&err))),
    }
}

fn finish(result: Result<Value, Box<Response>>) -> Response {
    match result {
        Ok(value) => json_at(200, value),
        Err(response) => *response,
    }
}

fn route_response(err: RouteError) -> Response {
    match err {
        RouteError::Pack(err) => pack_response(&err),
        RouteError::Http { status, body } => json_at(status, body),
    }
}

fn pack_response(err: &PackError) -> Response {
    json_at(err.code.status(), err.to_json())
}

fn unavailable() -> Response {
    json_at(503, json!({"error": "context_pack_library_unavailable"}))
}

fn failed_join(err: &tokio::task::JoinError) -> Response {
    json_at(
        500,
        json!({"error": "context_pack_failed", "message": err.to_string()}),
    )
}

pub(crate) fn json_at(status: u16, value: Value) -> Response {
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (code, Json(value)).into_response()
}
