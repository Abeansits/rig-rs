//! Temporary `GET /api/rigs/{id}/chat/watch` SSE stream.
//!
//! The frame is `{id, type, payload}` with the message id on the SSE `id:` line
//! so `Last-Event-ID` can resume. `/api/events` tails the event log with a
//! numeric id and a seat payload, so chat messages stay on this route.

use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::response::IntoResponse;
use axum::response::sse::{Event, Sse};
use crewmux_api::{CHAT_MESSAGE_TYPE, ChatWatchEvent};
use crewmux_core::chat::{ChatMessage, ChatRepo, HistoryQuery};
use crewmux_types::{ChatCursor, RigId};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::Stream;
use tokio_util::sync::CancellationToken;

use crate::state::App;

use super::http::wire;

const INITIAL_LIMIT: u32 = 20;
const PAGE_LIMIT: u32 = 100;

pub(crate) fn attach(app: App, rig: RigId, after: Option<ChatCursor>) -> axum::response::Response {
    let cancel = app.cancel.child_token();
    let bus = app.chat.subscribe();
    let tasks = app.tasks.clone();
    let (tx, rx) = mpsc::channel(32);
    let task_cancel = cancel.clone();
    tasks.spawn(async move {
        produce(app, rig, after, bus, tx, task_cancel).await;
    });
    Sse::new(WatchBody { rx, cancel }).into_response()
}

struct WatchBody {
    rx: mpsc::Receiver<Event>,
    cancel: CancellationToken,
}

impl Drop for WatchBody {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl Stream for WatchBody {
    type Item = Result<Event, Infallible>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().rx.poll_recv(cx).map(|event| event.map(Ok))
    }
}

async fn produce(
    app: App,
    rig: RigId,
    after: Option<ChatCursor>,
    mut bus: broadcast::Receiver<RigId>,
    tx: mpsc::Sender<Event>,
    cancel: CancellationToken,
) {
    let mut last = after.clone();
    if !send_existing(&app, &rig, after.as_ref(), &tx, &mut last).await {
        return;
    }
    loop {
        let noticed = tokio::select! {
            () = cancel.cancelled() => return,
            event = bus.recv() => event,
        };
        match noticed {
            Ok(id) if id != rig => {}
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {
                if !send_after(&app, &rig, &tx, &mut last).await {
                    return;
                }
            }
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

async fn send_existing(
    app: &App,
    rig: &RigId,
    after: Option<&ChatCursor>,
    tx: &mpsc::Sender<Event>,
    last: &mut Option<ChatCursor>,
) -> bool {
    if after.is_none() {
        return send_query(app, newest(rig), tx, last).await;
    }
    send_forward(app, rig, tx, last).await
}

async fn send_after(
    app: &App,
    rig: &RigId,
    tx: &mpsc::Sender<Event>,
    last: &mut Option<ChatCursor>,
) -> bool {
    if last.is_none() {
        *last = ChatCursor::new("0").ok();
    }
    send_forward(app, rig, tx, last).await
}

async fn send_forward(
    app: &App,
    rig: &RigId,
    tx: &mpsc::Sender<Event>,
    last: &mut Option<ChatCursor>,
) -> bool {
    for _ in 0..50 {
        let before = last.clone();
        if !send_query(app, forward(rig, last.clone()), tx, last).await {
            return false;
        }
        if last.as_ref().map(ChatCursor::as_str) == before.as_ref().map(ChatCursor::as_str) {
            return true;
        }
    }
    true
}

async fn send_query(
    app: &App,
    query: HistoryQuery,
    tx: &mpsc::Sender<Event>,
    last: &mut Option<ChatCursor>,
) -> bool {
    let page = match read_history(app, query).await {
        Ok(page) => page,
        Err(err) => {
            tracing::warn!(error = %err, "chat watch history");
            return false;
        }
    };
    for message in page.messages {
        if !send_one(tx, &message).await {
            return false;
        }
        *last = Some(ChatCursor::from_chat_id(message.id));
    }
    true
}

async fn send_one(tx: &mpsc::Sender<Event>, message: &ChatMessage) -> bool {
    let event = frame(message);
    tx.send(event).await.is_ok()
}

fn frame(message: &ChatMessage) -> Event {
    let payload = wire(message);
    let id = payload.id.clone();
    let envelope = ChatWatchEvent {
        id: id.clone(),
        event_type: CHAT_MESSAGE_TYPE.to_owned(),
        payload,
    };
    let data = serde_json::to_string(&envelope).unwrap_or_else(|_| "{}".to_owned());
    Event::default().id(id).event(CHAT_MESSAGE_TYPE).data(data)
}

fn newest(rig: &RigId) -> HistoryQuery {
    HistoryQuery {
        rig: *rig,
        topic: None,
        after: None,
        since: None,
        sender: None,
        limit: INITIAL_LIMIT,
    }
}

fn forward(rig: &RigId, after: Option<ChatCursor>) -> HistoryQuery {
    HistoryQuery {
        rig: *rig,
        topic: None,
        after,
        since: None,
        sender: None,
        limit: PAGE_LIMIT,
    }
}

async fn read_history(
    app: &App,
    query: HistoryQuery,
) -> Result<crewmux_core::chat::HistoryPage, crewmux_core::chat::ChatError> {
    let store = std::sync::Arc::clone(&app.store);
    match tokio::task::spawn_blocking(move || {
        store
            .read(move |conn| {
                crewmux_store::Db::new(conn)
                    .history(&query)
                    .map_err(crewmux_store::Error::from)
            })
            .map_err(crewmux_store::Error::into_chat)
    })
    .await
    {
        Ok(page) => page,
        Err(err) => Err(crewmux_core::chat::ChatError::Store(err.to_string())),
    }
}
