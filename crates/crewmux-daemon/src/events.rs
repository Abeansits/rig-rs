//! `GET /api/events` over SSE and WebSocket, plus `/api/queue/watch`.
//!
//! This hub-backed handler is the canonical stream. It replaces the interim
//! event-log tail on the same path and the same `{id, type, payload}` envelope.

use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use crewmux_api::{StreamEvent, StreamKind};
use futures_core::Stream;
use tokio::sync::mpsc;

use crate::hub::Missed;
use crate::state::App;

/// Route the TUI opens. The interim tail used this path.
pub(crate) const PATH: &str = "/api/events";

/// Which hub events a tail keeps. Gap is always kept so a resume can see a hole.
#[derive(Clone, Copy)]
pub(crate) enum Tail {
    /// Every kind.
    All,
    /// Queue changes.
    Queue,
    /// Stream items. This is `/api/stream/watch`, not a second bus.
    Stream,
}

pub(crate) async fn events(State(app): State<App>, req: Request) -> Response {
    app.touch();
    dispatch(app, req, Tail::All).await
}

pub(crate) async fn watch(State(app): State<App>, req: Request) -> Response {
    app.touch();
    dispatch(app, req, Tail::Queue).await
}

pub(crate) async fn stream_watch(State(app): State<App>, req: Request) -> Response {
    app.touch();
    dispatch(app, req, Tail::Stream).await
}

async fn dispatch(app: App, req: Request, tail: Tail) -> Response {
    let (mut parts, body) = req.into_parts();
    let last = resume_id(&parts.headers, parts.uri.query());
    let upgrade = parts
        .headers
        .get(header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"));
    if upgrade {
        return match WebSocketUpgrade::from_request_parts(&mut parts, &app).await {
            Ok(ws) => ws.on_upgrade(move |socket| ws_pump(socket, app, last, tail)),
            Err(err) => err.into_response(),
        };
    }
    drop(parts);
    drop(body);
    sse(&app, last, tail)
}

fn passes(tail: Tail, kind: StreamKind) -> bool {
    match tail {
        Tail::All => true,
        Tail::Queue => matches!(kind, StreamKind::QueueChanged | StreamKind::Gap),
        Tail::Stream => matches!(kind, StreamKind::StreamEmitted | StreamKind::Gap),
    }
}

fn sse(app: &App, last: Option<u64>, tail: Tail) -> Response {
    let (tx, rx) = mpsc::channel(32);
    let hub = std::sync::Arc::clone(&app.hub);
    let cancel = app.cancel.clone();
    app.tasks.spawn(async move {
        let replay = replay(&hub, last, tail).await;
        for event in replay {
            if tx.send(format_sse(&event)).await.is_err() {
                return;
            }
        }
        let mut live = hub.subscribe();
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                event = live.recv() => {
                    match event {
                        Ok(event) if passes(tail, event.kind) => {
                            if tx.send(format_sse(&event)).await.is_err() {
                                break;
                            }
                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            let gap = StreamEvent { id: 0, kind: StreamKind::Gap, payload: None };
                            if tx.send(format_sse(&gap)).await.is_err() {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }
    });
    let stream = ReceiverStream { rx };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn ws_pump(mut socket: WebSocket, app: App, last: Option<u64>, tail: Tail) {
    for event in replay(&app.hub, last, tail).await {
        let text = serde_json::to_string(&event).unwrap_or_default();
        if socket.send(Message::Text(text.into())).await.is_err() {
            return;
        }
    }
    let mut live = app.hub.subscribe();
    loop {
        tokio::select! {
            () = app.cancel.cancelled() => break,
            event = live.recv() => {
                let Ok(event) = event else { break };
                if !passes(tail, event.kind) {
                    continue;
                }
                let text = serde_json::to_string(&event).unwrap_or_default();
                if socket.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
        }
    }
}

async fn replay(hub: &crate::hub::EventHub, last: Option<u64>, tail: Tail) -> Vec<StreamEvent> {
    let mut out = Vec::new();
    match hub.missed(last).await {
        Missed::CaughtUp => {}
        Missed::Replay(events) => out.extend(events),
        Missed::Gap(events) => {
            // No buffered event to name. The id is the head itself, so the next
            // real event is `head + 1`. A client that resumes from this gap
            // still receives that event.
            let id = events.first().map_or_else(|| hub.head(), |event| event.id);
            out.push(StreamEvent {
                id,
                kind: StreamKind::Gap,
                payload: None,
            });
            out.extend(events);
        }
    }
    out.retain(|event| passes(tail, event.kind));
    out
}

fn resume_id(headers: &axum::http::HeaderMap, query: Option<&str>) -> Option<u64> {
    let header = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(bounded_id);
    header.or_else(|| query_id(query))
}

/// Numeric event ids are short. A longer header or query is ignored rather than parsed.
fn bounded_id(text: &str) -> Option<u64> {
    if text.len() > 20 {
        None
    } else {
        text.parse().ok()
    }
}

fn query_id(query: Option<&str>) -> Option<u64> {
    let query = query?;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        if key == "last_event_id" {
            return bounded_id(value);
        }
    }
    None
}

fn format_sse(event: &StreamEvent) -> String {
    let data = serde_json::to_string(event).unwrap_or_else(|_| "{}".to_owned());
    format!(
        "id: {}\nevent: {}\ndata: {data}\n\n",
        event.id,
        event.kind.as_str()
    )
}

struct ReceiverStream {
    rx: mpsc::Receiver<String>,
}

impl Stream for ReceiverStream {
    type Item = Result<String, Infallible>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().rx.poll_recv(cx).map(|item| item.map(Ok))
    }
}

pub(crate) async fn terminal(State(app): State<App>, req: Request) -> Response {
    app.touch();
    let seat = query_seat(req.uri().query()).unwrap_or_else(|| "seat".to_owned());
    let (mut parts, body) = req.into_parts();
    match WebSocketUpgrade::from_request_parts(&mut parts, &app).await {
        Ok(upgrade) => upgrade.on_upgrade(move |socket| terminal_pump(socket, app, seat)),
        Err(err) => {
            drop(parts);
            drop(body);
            err.into_response()
        }
    }
}

fn query_seat(query: Option<&str>) -> Option<String> {
    let query = query?;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        if key == "seat" && !value.is_empty() {
            return Some(value.to_owned());
        }
    }
    None
}

async fn terminal_pump(mut socket: WebSocket, app: App, seat: String) {
    while let Some(message) = socket.recv().await {
        let Ok(message) = message else {
            break;
        };
        if matches!(message, Message::Text(_) | Message::Binary(_)) {
            app.taps.record_name(&seat, std::time::Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Tail, replay};
    use crate::hub::EventHub;
    use crewmux_api::StreamKind;

    #[tokio::test]
    async fn last_event_id_past_the_head_is_a_gap() {
        let hub = EventHub::new();
        let missed = replay(&hub, Some(3), Tail::All).await;
        let gap = missed.first().expect("gap");
        assert_eq!(gap.kind, StreamKind::Gap);
        assert_eq!(gap.id, hub.head());
        assert_ne!(gap.id, hub.head().saturating_add(1));
        assert_eq!(hub.head(), 0);
        assert_eq!(missed.len(), 1);

        let published = hub.publish(StreamKind::Activity, None).await;
        let caught = replay(&hub, Some(published.id), Tail::All).await;
        assert_eq!(caught.len(), 0);
        let ahead = replay(&hub, Some(published.id.saturating_add(2)), Tail::All).await;
        let gap = ahead.first().expect("ahead");
        assert_eq!(gap.kind, StreamKind::Gap);
        assert_eq!(gap.id, hub.head());
        assert_ne!(gap.id, hub.head().saturating_add(1));
        assert!(ahead.iter().all(|event| event.kind == StreamKind::Gap));
    }

    #[tokio::test]
    async fn stream_tail_keeps_emitted_items_and_resumes() {
        let hub = EventHub::new();
        let queue = hub.publish(StreamKind::QueueChanged, None).await;
        let emitted = hub
            .publish(
                StreamKind::StreamEmitted,
                Some(crewmux_api::StreamPayload {
                    stream_id: Some("item-1".to_owned()),
                    ..crewmux_api::StreamPayload::default()
                }),
            )
            .await;
        let all = replay(&hub, None, Tail::Stream).await;
        assert!(
            all.iter()
                .all(|event| event.kind != StreamKind::QueueChanged)
        );
        assert_eq!(all.last().map(|event| event.id), Some(emitted.id));
        let resumed = replay(&hub, Some(emitted.id), Tail::Stream).await;
        assert_eq!(resumed, []);
        assert_eq!(queue.kind, StreamKind::QueueChanged);
    }
}
