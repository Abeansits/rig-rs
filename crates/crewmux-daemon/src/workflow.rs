//! Boot sweep and the paired queue/workflow commit.
//!
//! The two in-memory documents swap while both locks are held. The locks are
//! dropped before the store write is awaited.

use std::sync::Arc;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use crewmux_messaging::QueueBook;
use crewmux_orchestration::{Engine, World, boot_sweep, frontier_block, install_builtins};
use serde_json::{Value, json};

use crate::error::Error;
use crate::http::fail;
use crate::launch::now_ts;
use crate::state::{App, join_err};

/// Logged when a restart finds no live instance.
pub(crate) const BOOT_EMPTY: &str = "workflow boot sweep: 0 in-flight instances (no-op)";

/// Load the document, install built-ins, sweep, and commit the pair.
///
/// # Errors
///
/// Returns [`Error`] when the document cannot be read or the commit fails.
pub(crate) async fn boot(app: &App, mut book: QueueBook) -> Result<(), Error> {
    let store = Arc::clone(&app.store);
    let loaded =
        tokio::task::spawn_blocking(move || store.read(crewmux_store::load_workflow_document))
            .await
            .map_err(join_err)??;
    let mut engine = match loaded {
        Some(text) => serde_json::from_str(&text)
            .map_err(|err| Error::Message(format!("workflow document: {err}")))?,
        None => Engine::default(),
    };
    let now = now_ts();
    install_builtins(&mut engine, now);
    let sweep = boot_sweep(&mut engine, &mut book, &World::default(), now);
    if sweep.visited == 0 {
        tracing::info!("{BOOT_EMPTY}");
    }
    let doc = serde_json::to_string(&engine).map_err(|err| Error::Message(err.to_string()))?;
    swap_pair(app, book.clone(), engine).await;
    app.commit_documented(book, Some(doc)).await
}

/// Refuse a terminal close of a live frontier packet.
pub(crate) async fn guard_close(app: &App, qitem: &str, next: &str) -> Option<Response> {
    let engine = app.workflow.lock().await.clone();
    frontier_block(&engine, qitem, next).map(rejected)
}

/// Run `f` on a consistent pair and publish both documents.
pub(crate) async fn mutate_pair<T>(
    app: &App,
    f: impl FnOnce(&Engine, &QueueBook) -> Result<(Engine, QueueBook, T), crewmux_orchestration::Error>,
) -> Result<T, Box<Response>> {
    let (previous_book, previous_engine, next_book, doc, epoch, value) = loop {
        if let (Ok(mut queue), Ok(mut engine)) = (app.queue.try_lock(), app.workflow.try_lock()) {
            let previous_book = queue.clone();
            let previous_engine = engine.clone();
            let (next_engine, next_book, value) = match f(&engine, &queue) {
                Ok(triple) => triple,
                Err(err) => return Err(Box::new(rejected(err))),
            };
            let doc = match serde_json::to_string(&next_engine) {
                Ok(doc) => doc,
                Err(err) => return Err(Box::new(fail(Error::Message(err.to_string())))),
            };
            let epoch = next_book.epoch();
            *queue = next_book.clone();
            *engine = next_engine;
            break (previous_book, previous_engine, next_book, doc, epoch, value);
        }
        tokio::task::yield_now().await;
    };
    if let Err(err) = app.commit_documented(next_book, Some(doc)).await {
        restore(app, epoch, previous_book, previous_engine).await;
        return Err(Box::new(fail(err)));
    }
    Ok(value)
}

/// Seats the daemon has stored. Missing rows leave the world empty.
pub(crate) async fn world(app: &App) -> World {
    let store = Arc::clone(&app.store);
    let rows = tokio::task::spawn_blocking(move || store.read(crewmux_store::list_seat_coords))
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    let mut world = World::default();
    for row in rows {
        world.rigs.insert(row.rig.clone());
        let coord = format!("{}@{}", row.name, row.rig);
        world.members.insert(coord.clone());
        world.runtimes.insert(coord, row.runtime);
    }
    world
}

/// Read a spec off the worker thread.
pub(crate) async fn read_spec(path: &str) -> Result<String, crewmux_orchestration::Error> {
    let owned = path.to_owned();
    let read = tokio::task::spawn_blocking(move || {
        let file = std::path::Path::new(&owned);
        if !file.is_file() {
            return Err(missing_spec(&owned));
        }
        std::fs::read_to_string(file).map_err(|err| crewmux_orchestration::Error::Rejected {
            code: "spec_file_missing",
            message: format!("workflow spec file not found at {owned}: {err}"),
            status: 404,
        })
    })
    .await;
    match read {
        Ok(result) => result,
        Err(err) => Err(crewmux_orchestration::Error::Rejected {
            code: "spec_file_missing",
            message: err.to_string(),
            status: 500,
        }),
    }
}

fn missing_spec(path: &str) -> crewmux_orchestration::Error {
    crewmux_orchestration::Error::Rejected {
        code: "spec_file_missing",
        message: format!("workflow spec file not found at {path}"),
        status: 404,
    }
}

pub(crate) fn rejected(err: crewmux_orchestration::Error) -> Response {
    match err {
        crewmux_orchestration::Error::Rejected {
            code,
            message,
            status,
        } => {
            let code_status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST);
            (
                code_status,
                Json(json!({"error": code, "message": message})),
            )
                .into_response()
        }
        crewmux_orchestration::Error::Crashed(point) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "crashed", "message": format!("workflow crashed at {point}")})),
        )
            .into_response(),
    }
}

pub(crate) fn missing_field(name: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error": format!("{name} is required")})),
    )
        .into_response()
}

pub(crate) fn json_status(status: StatusCode, value: Value) -> Response {
    (status, Json(value)).into_response()
}

async fn swap_pair(app: &App, book: QueueBook, engine: Engine) {
    loop {
        if let (Ok(mut queue), Ok(mut slot)) = (app.queue.try_lock(), app.workflow.try_lock()) {
            *queue = book;
            *slot = engine;
            return;
        }
        tokio::task::yield_now().await;
    }
}

async fn restore(app: &App, epoch: u64, book: QueueBook, engine: Engine) {
    loop {
        if let (Ok(mut queue), Ok(mut slot)) = (app.queue.try_lock(), app.workflow.try_lock()) {
            if queue.epoch() == epoch {
                *queue = book;
                *slot = engine;
            }
            return;
        }
        tokio::task::yield_now().await;
    }
}
