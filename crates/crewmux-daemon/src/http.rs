//! HTTP/1.1 routes on the Unix socket.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::map_response;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use crewmux_api::{
    AttachInfo, AttachRequest, CaptureRequest, CaptureSeat, CreateRigUp, DownRequest, Endpoint,
    Healthz, ListSeats, RigDown,
};
use serde_json::{Value, json};

use crate::error::Error;
use crate::launch::bring_up;
use crate::query::{attach_argv, capture, down, list_seats};
use crate::state::App;

pub(crate) fn router(app: App) -> Router {
    Router::new()
        .route(Healthz::PATH, get(health))
        .route(CreateRigUp::PATH, post(up))
        .route(ListSeats::PATH, get(ps))
        .route(CaptureSeat::PATH, post(capture_seat))
        .route(AttachInfo::PATH, post(attach))
        .route(RigDown::PATH, post(rig_down))
        .route(crate::events::PATH, get(crate::events::events))
        .route("/shutdown", post(shutdown))
        .layer(map_response(connection_close))
        .with_state(app)
}

async fn health(State(app): State<App>) -> Json<Value> {
    app.touch();
    Json(json!({"status": "ok", "pid": std::process::id()}))
}

async fn up(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let text = match String::from_utf8(body.to_vec()) {
        Ok(text) => text,
        Err(err) => return fail(Error::Message(err.to_string())),
    };
    match bring_up(&app, &text).await {
        Ok(value) => ok(value),
        Err(err) => fail(err),
    }
}

async fn ps(State(app): State<App>) -> Response {
    app.touch();
    match list_seats(&app).await {
        Ok(value) => ok(value),
        Err(err) => fail(err),
    }
}

async fn capture_seat(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let request: CaptureRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(err) => return fail(Error::Message(err.to_string())),
    };
    match capture(&app, &request.seat, request.lines).await {
        Ok(value) => ok(value),
        Err(err) => fail(err),
    }
}

async fn attach(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let request: AttachRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(err) => return fail(Error::Message(err.to_string())),
    };
    match attach_argv(&app, &request.seat).await {
        Ok(argv) => ok(json!({"argv": argv})),
        Err(err) => fail(err),
    }
}

async fn rig_down(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let rig = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<DownRequest>(&body) {
            Ok(request) => request.rig,
            Err(err) => return fail(Error::Message(err.to_string())),
        }
    };
    match down(&app, rig.as_deref()).await {
        Ok(value) => ok(value),
        Err(err) => fail(err),
    }
}

async fn shutdown(State(app): State<App>) -> Response {
    app.touch();
    app.cancel.cancel();
    ok(json!({"status": "stopping"}))
}

async fn connection_close(mut response: Response) -> Response {
    response.headers_mut().insert(
        axum::http::header::CONNECTION,
        HeaderValue::from_static("close"),
    );
    response
}

fn ok(value: Value) -> Response {
    (StatusCode::OK, Json(value)).into_response()
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "handlers pass Error by value and Display does not consume it"
)]
fn fail(err: Error) -> Response {
    let message = err.to_string();
    let status = if message.contains("not found") {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::BAD_REQUEST
    };
    (status, Json(json!({"error": message}))).into_response()
}
