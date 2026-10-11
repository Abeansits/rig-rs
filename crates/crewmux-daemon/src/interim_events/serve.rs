//! TEMPORARY `GET /api/events` handler.
//!
//! Slice 4 owns this route, the transcript writer, and the backlog. This handler
//! stays on [`crewmux_api::Events::PATH`] and writes the same envelope. It does
//! not store events. A resume (`Last-Event-ID` or `last_event_id`) gets one
//! `gap` frame, then live events. Delete this module when slice 4 mounts its own.

use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use axum::Router;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use futures_util::stream::Stream;
use serde::Deserialize;
use tokio::net::UnixListener;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crewmux_api::{Endpoint, Events, MAX_SUN_PATH_BYTES, StreamEvent, path_bytes};

use super::error::Error;
use super::hub::EventHub;

/// Bind `path` mode `0600` and serve [`Events::PATH`] until `cancel` fires.
///
/// # Errors
///
/// Returns [`Error::SunPath`] when `path` exceeds the guard, or [`Error::Bind`]
/// / [`Error::Serve`] when the socket cannot be used.
pub async fn serve(path: &Path, hub: EventHub, cancel: CancellationToken) -> Result<(), Error> {
    let path = check_sun(path)?;
    prepare_dir(&path).await?;
    if tokio::fs::try_exists(&path)
        .await
        .map_err(|source| bind_err(&path, source))?
    {
        tokio::fs::remove_file(&path)
            .await
            .map_err(|source| bind_err(&path, source))?;
    }
    let listener = UnixListener::bind(&path).map_err(|source| bind_err(&path, source))?;
    set_mode(&path, 0o600).await?;
    info!(path = %path.display(), "event stream listening");
    let app = Router::new()
        .route(Events::PATH, get(events))
        .with_state(hub);
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            cancel.cancelled().await;
        })
        .await
        .map_err(Error::Serve)?;
    Ok(())
}

#[derive(Debug, Default, Deserialize)]
struct Resume {
    #[serde(default)]
    last_event_id: Option<String>,
}

struct Live {
    gap: Option<StreamEvent>,
    rx: broadcast::Receiver<StreamEvent>,
}

async fn events(
    State(hub): State<EventHub>,
    headers: HeaderMap,
    Query(resume): Query<Resume>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let live = Live {
        rx: hub.subscribe(),
        gap: resume_gap(&hub, &headers, &resume),
    };
    Sse::new(futures_util::stream::unfold(live, next_event)).keep_alive(KeepAlive::default())
}

fn resume_gap(hub: &EventHub, headers: &HeaderMap, resume: &Resume) -> Option<StreamEvent> {
    let header = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty());
    let query = resume
        .last_event_id
        .as_deref()
        .filter(|value| !value.is_empty());
    if header.is_some() || query.is_some() {
        Some(hub.gap())
    } else {
        None
    }
}

async fn next_event(mut live: Live) -> Option<(Result<Event, std::convert::Infallible>, Live)> {
    if let Some(event) = live.gap.take() {
        return Some((Ok(frame(&event)), live));
    }
    loop {
        match live.rx.recv().await {
            Ok(event) => return Some((Ok(frame(&event)), live)),
            Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => return None,
        }
    }
}

fn frame(event: &StreamEvent) -> Event {
    let data = serde_json::to_string(event).unwrap_or_default();
    Event::default().id(event.id.to_string()).data(data)
}

fn check_sun(path: &Path) -> Result<PathBuf, Error> {
    let bytes = path_bytes(path);
    if bytes > MAX_SUN_PATH_BYTES {
        return Err(Error::SunPath {
            path: path.to_path_buf(),
            bytes,
            limit: MAX_SUN_PATH_BYTES,
        });
    }
    Ok(path.to_path_buf())
}

async fn prepare_dir(path: &Path) -> Result<(), Error> {
    let Some(parent) = path.parent() else {
        return Err(bind_err(
            path,
            io::Error::new(io::ErrorKind::InvalidInput, "socket path has no directory"),
        ));
    };
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|source| bind_err(path, source))?;
    set_mode(parent, 0o700).await?;
    Ok(())
}

async fn set_mode(path: &Path, mode: u32) -> Result<(), Error> {
    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|source| bind_err(path, source))?;
    let mut perms = meta.permissions();
    // `chmod` keeps the file type; mask anyway so a later API change cannot drop it.
    perms.set_mode((meta.permissions().mode() & !0o777) | (mode & 0o777));
    tokio::fs::set_permissions(path, perms)
        .await
        .map_err(|source| bind_err(path, source))?;
    Ok(())
}

fn bind_err(path: &Path, source: io::Error) -> Error {
    Error::Bind {
        path: path.to_path_buf(),
        source,
    }
}
