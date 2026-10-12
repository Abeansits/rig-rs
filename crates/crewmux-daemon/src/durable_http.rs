//! Snapshot, restore, restore-check, crash-cart, checkpoint, and edge routes.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use axum::routing::{get, post};
use crewmux_durability::parse_posture;
use crewmux_store::EdgeRow;
use crewmux_types::LaunchPosture;
use serde::Deserialize;
use serde_json::json;

use crate::checkpoint::{CheckpointDraft, show_checkpoint, write_checkpoint};
use crate::error::Error;
use crate::state::{App, join_err};

pub(crate) fn routes() -> Router<App> {
    Router::new()
        .route("/api/snapshot/create", post(snapshot_create))
        .route("/api/snapshot/list", post(snapshot_list))
        .route("/api/snapshot/show", post(snapshot_show))
        .route("/api/restore", post(restore))
        .route("/api/restore-check", get(restore_check))
        .route("/api/crash-cart/restore", post(crash_cart))
        .route("/api/checkpoint", post(checkpoint_write))
        .route("/api/checkpoint/show", post(checkpoint_show))
        .route("/api/edges", post(put_edge))
        .route("/api/kernel", post(mark_kernel))
}

async fn snapshot_create(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let parsed = match parse::<RigBody>(&body) {
        Ok(parsed) => parsed,
        Err(err) => return crate::http::fail(err),
    };
    match crate::snapshot::create(&app, &parsed.rig).await {
        Ok(value) => crate::http::ok(value),
        Err(err) => crate::http::fail(err),
    }
}

async fn snapshot_list(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let parsed = match parse::<RigBody>(&body) {
        Ok(parsed) => parsed,
        Err(err) => return crate::http::fail(err),
    };
    match crate::snapshot::list(&app, &parsed.rig).await {
        Ok(value) => crate::http::ok(value),
        Err(err) => crate::http::fail(err),
    }
}

async fn snapshot_show(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let parsed = match parse::<ShowBody>(&body) {
        Ok(parsed) => parsed,
        Err(err) => return crate::http::fail(err),
    };
    match crate::snapshot::show(&app, &parsed.rig, &parsed.id).await {
        Ok(value) => crate::http::ok(value),
        Err(err) => crate::http::fail(err),
    }
}

async fn restore(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let parsed = match parse::<RestoreBody>(&body) {
        Ok(parsed) => parsed,
        Err(err) => return crate::http::fail(err),
    };
    let posture = parsed
        .posture
        .as_deref()
        .map_or(LaunchPosture::Auto, parse_posture);
    match crate::restore::restore_rig(&app, &parsed.rig, parsed.snapshot.as_deref(), posture).await
    {
        Ok(value) => crate::http::ok(value),
        Err(err) => crate::http::fail(err),
    }
}

async fn restore_check(State(app): State<App>) -> Response {
    app.touch();
    match crate::check_run::report(&app).await {
        Ok(value) => crate::http::ok(value),
        Err(err) => crate::http::fail(err),
    }
}

async fn crash_cart(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let cancel = if body.is_empty() {
        false
    } else {
        match parse::<CancelBody>(&body) {
            Ok(parsed) => parsed.cancel,
            Err(err) => return crate::http::fail(err),
        }
    };
    match crate::cart::crash_cart(&app, cancel).await {
        Ok(value) => crate::http::ok(value),
        Err(err) => crate::http::fail(err),
    }
}

async fn checkpoint_write(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let parsed = match parse::<CheckpointBody>(&body) {
        Ok(parsed) => parsed,
        Err(err) => return crate::http::fail(err),
    };
    let draft = CheckpointDraft {
        seat: parsed.seat,
        summary: parsed.summary,
        current_task: parsed.current_task,
        next_step: parsed.next_step,
        blocked_on: parsed.blocked_on,
        key_artifacts: parsed.key_artifacts.unwrap_or_default(),
        confidence: parsed.confidence,
    };
    match write_checkpoint(&app, draft).await {
        Ok(value) => crate::http::ok(value),
        Err(err) => crate::http::fail(err),
    }
}

async fn checkpoint_show(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let seat = if body.is_empty() {
        return crate::http::fail(Error::Message("seat is required".to_owned()));
    } else {
        match parse::<SeatBody>(&body) {
            Ok(parsed) => parsed.seat,
            Err(err) => return crate::http::fail(err),
        }
    };
    match show_checkpoint(&app, &seat).await {
        Ok(value) => crate::http::ok(value),
        Err(err) => crate::http::fail(err),
    }
}

async fn put_edge(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let parsed = match parse::<EdgeBody>(&body) {
        Ok(parsed) => parsed,
        Err(err) => return crate::http::fail(err),
    };
    match write_edge(&app, &parsed).await {
        Ok(()) => crate::http::ok(json!({"status": "ok"})),
        Err(err) => crate::http::fail(err),
    }
}

async fn mark_kernel(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let parsed = match parse::<RigBody>(&body) {
        Ok(parsed) => parsed,
        Err(err) => return crate::http::fail(err),
    };
    match flag_kernel(&app, &parsed.rig).await {
        Ok(()) => crate::http::ok(json!({"status": "ok", "rig": parsed.rig})),
        Err(err) => crate::http::fail(err),
    }
}

async fn write_edge(app: &App, body: &EdgeBody) -> Result<(), Error> {
    let (rig_id, _) = crate::snapshot::load_rig(app, &body.rig).await?;
    let source = crate::query::find_seat(app, &body.source).await?;
    let target = crate::query::find_seat(app, &body.target).await?;
    let edge = EdgeRow {
        source_id: source.id.to_string(),
        target_id: target.id.to_string(),
        kind: body.kind.clone(),
    };
    let rig = rig_id.to_string();
    let store = Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || {
        store.write(move |conn| crewmux_store::put_edge(conn, &rig, &edge))
    })
    .await
    .map_err(join_err)?
    .map_err(Error::from)
}

async fn flag_kernel(app: &App, name: &str) -> Result<(), Error> {
    let (rig_id, _) = crate::snapshot::load_rig(app, name).await?;
    let rig = rig_id.to_string();
    let store = Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || {
        store.write(move |conn| crewmux_store::mark_kernel(conn, &rig))
    })
    .await
    .map_err(join_err)?
    .map_err(Error::from)
}

fn parse<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, Error> {
    serde_json::from_slice(body).map_err(|err| Error::Message(err.to_string()))
}

#[derive(Deserialize)]
struct RigBody {
    rig: String,
}

#[derive(Deserialize)]
struct ShowBody {
    rig: String,
    id: String,
}

#[derive(Deserialize)]
struct RestoreBody {
    rig: String,
    snapshot: Option<String>,
    posture: Option<String>,
}

#[derive(Deserialize)]
struct CancelBody {
    #[serde(default)]
    cancel: bool,
}

#[derive(Deserialize)]
struct SeatBody {
    seat: String,
}

#[derive(Deserialize)]
struct EdgeBody {
    rig: String,
    source: String,
    target: String,
    kind: String,
}

#[derive(Deserialize)]
struct CheckpointBody {
    seat: String,
    summary: String,
    current_task: Option<String>,
    next_step: Option<String>,
    blocked_on: Option<String>,
    key_artifacts: Option<Vec<String>>,
    confidence: Option<String>,
}
