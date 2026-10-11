// Derived from openrig packages/daemon/src/routes/chat.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! `POST /send`, `GET /history`, `GET /watch`, `POST /topic`, `POST /clear`.

use std::str::FromStr;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use crewmux_api::{ChatWire, RigSummaryRow};
use crewmux_core::RigRepo;
use crewmux_core::chat::{ChatError, ChatMessage, ChatRepo, HistoryQuery, NewChat, NewTopic};
use crewmux_core::chat::{format_sqlite_utc, parse_since, rejected_since};
use crewmux_messaging::chat::{SeatProof, resolve_sender};
use crewmux_store::{Db, seat_token};
use crewmux_types::{ChatCursor, RigId, SEAT_ID_HEADER, SEAT_TOKEN_HEADER, SenderRef, TopicId};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::launch::now_ts;
use crate::state::App;

use super::watch;

pub(crate) fn routes() -> Router<App> {
    Router::new()
        .route("/api/rigs/summary", get(summary))
        .route("/api/rigs/{rig_id}/chat/send", post(send))
        .route("/api/rigs/{rig_id}/chat/history", get(history))
        .route("/api/rigs/{rig_id}/chat/watch", get(watch_route))
        .route("/api/rigs/{rig_id}/chat/topic", post(topic))
        .route("/api/rigs/{rig_id}/chat/clear", post(clear))
}

async fn summary(State(app): State<App>) -> Response {
    app.touch();
    let rigs = match crate::db::read(&app.store, |conn| conn.list()).await {
        Ok(rigs) => rigs,
        Err(err) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    };
    let mut rows: Vec<RigSummaryRow> = rigs
        .into_iter()
        .map(|rig| RigSummaryRow {
            id: rig.id.to_string(),
            name: rig.name.as_str().to_owned(),
        })
        .collect();
    rows.sort_by(|left, right| left.name.cmp(&right.name));
    Json(rows).into_response()
}

async fn send(
    State(app): State<App>,
    Path(rig_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    app.touch();
    let Some(rig) = parse_rig(&rig_id) else {
        return json_error(StatusCode::BAD_REQUEST, "invalid rig id");
    };
    let obj = object(&body);
    let Some(text) = obj
        .get("body")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    else {
        return json_error(StatusCode::BAD_REQUEST, "Missing body");
    };
    let claimed = obj.get("sender").and_then(Value::as_str);
    let sender = match authenticate(&app, &headers, claimed).await {
        Ok(sender) => sender,
        Err(err) => return err.response(),
    };
    let message = NewChat {
        rig,
        sender,
        body: text.to_owned(),
        at: now_ts(),
    };
    match write_chat(&app, move |db| db.post(message)).await {
        Ok(message) => {
            app.chat.notify(rig);
            created(&message)
        }
        Err(err) => chat_error(&err),
    }
}

async fn history(
    State(app): State<App>,
    Path(rig_id): Path<String>,
    Query(query): Query<HistoryQs>,
) -> Response {
    app.touch();
    let Some(rig) = parse_rig(&rig_id) else {
        return json_error(StatusCode::BAD_REQUEST, "invalid rig id");
    };
    let since = match since_bound(query.since.as_deref()) {
        Ok(since) => since,
        Err(message) => return json_error(StatusCode::BAD_REQUEST, message),
    };
    let topic = match topic_filter(query.topic.as_deref()) {
        Ok(topic) => topic,
        Err(err) => return err.response(),
    };
    if let Some(response) = unknown_topic(&app, &rig, topic.as_ref()).await {
        return response;
    }
    let after = match cursor_filter(query.after.as_deref()) {
        Ok(after) => after,
        Err(err) => return err.response(),
    };
    let sender = match sender_filter(query.sender.as_deref()) {
        Ok(sender) => sender,
        Err(err) => return err.response(),
    };
    let limit = match limit_filter(query.limit.as_deref()) {
        Ok(limit) => limit,
        Err(err) => return err.response(),
    };
    let query = HistoryQuery {
        rig,
        topic,
        after,
        since,
        sender,
        limit,
    };
    match read_chat(&app, move |db| db.history(&query)).await {
        Ok(page) => {
            let rows: Vec<ChatWire> = page.messages.iter().map(wire).collect();
            Json(rows).into_response()
        }
        Err(err) => chat_error(&err),
    }
}

async fn watch_route(
    State(app): State<App>,
    Path(rig_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    app.touch();
    let Some(rig) = parse_rig(&rig_id) else {
        return json_error(StatusCode::BAD_REQUEST, "invalid rig id");
    };
    let after = match cursor_filter(header_str(&headers, "last-event-id")) {
        Ok(after) => after,
        Err(err) => return err.response(),
    };
    watch::attach(app, rig, after)
}

async fn topic(
    State(app): State<App>,
    Path(rig_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    app.touch();
    let Some(rig) = parse_rig(&rig_id) else {
        return json_error(StatusCode::BAD_REQUEST, "invalid rig id");
    };
    let obj = object(&body);
    let Some(name) = obj
        .get("topic")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    else {
        return json_error(StatusCode::BAD_REQUEST, "Missing topic");
    };
    let Ok(name) = TopicId::new(name) else {
        return json_error(StatusCode::BAD_REQUEST, "invalid topic");
    };
    let text = obj.get("body").and_then(Value::as_str).unwrap_or("");
    let claimed = obj.get("sender").and_then(Value::as_str);
    let sender = match authenticate(&app, &headers, claimed).await {
        Ok(sender) => sender,
        Err(err) => return err.response(),
    };
    let marker = NewTopic {
        rig,
        sender,
        name,
        body: text.to_owned(),
        at: now_ts(),
    };
    match write_chat(&app, move |db| db.set_topic(marker)).await {
        Ok(message) => {
            app.chat.notify(rig);
            created(&message)
        }
        Err(err) => chat_error(&err),
    }
}

async fn clear(State(app): State<App>, Path(rig_id): Path<String>) -> Response {
    app.touch();
    let Some(rig) = parse_rig(&rig_id) else {
        return json_error(StatusCode::BAD_REQUEST, "invalid rig id");
    };
    match write_chat(&app, move |db| db.clear(&rig)).await {
        Ok(deleted) => Json(Cleared { ok: true, deleted }).into_response(),
        Err(err) => chat_error(&err),
    }
}

pub(crate) fn wire(message: &ChatMessage) -> ChatWire {
    ChatWire {
        id: message.id.to_string(),
        rig_id: message.rig.to_string(),
        sender: message.sender.as_str().to_owned(),
        kind: message.kind().to_owned(),
        body: message.body.clone(),
        topic: message
            .topic
            .as_ref()
            .map(|topic| topic.as_str().to_owned()),
        created_at: format_sqlite_utc(message.at),
    }
}

fn created(message: &ChatMessage) -> Response {
    (StatusCode::CREATED, Json(wire(message))).into_response()
}

async fn unknown_topic(app: &App, rig: &RigId, topic: Option<&TopicId>) -> Option<Response> {
    let topic = topic.cloned()?;
    let rig = *rig;
    let label = topic.as_str().to_owned();
    match read_chat(app, move |db| db.has_topic(&rig, &topic)).await {
        Ok(true) => None,
        Ok(false) => Some(json_error(
            StatusCode::BAD_REQUEST,
            format!("unknown topic: {label}"),
        )),
        Err(err) => Some(chat_error(&err)),
    }
}

async fn authenticate(
    app: &App,
    headers: &HeaderMap,
    claimed: Option<&str>,
) -> Result<SenderRef, Reject> {
    match presented(headers) {
        Presented::Partial => Err(Reject::new(StatusCode::BAD_REQUEST, "unknown sender")),
        Presented::None => resolve_sender(None, claimed).map_err(|err| chat_reject(&err)),
        Presented::Pair { seat, token } => {
            let stored = lookup_token(app, &seat).await?;
            let proof = SeatProof {
                seat_id: &seat,
                token: &token,
                stored_token: stored.as_deref(),
            };
            resolve_sender(Some(proof), claimed).map_err(|err| chat_reject(&err))
        }
    }
}

async fn lookup_token(app: &App, seat: &str) -> Result<Option<String>, Reject> {
    let seat_id = crewmux_types::SeatId::from_str(seat)
        .map_err(|_| Reject::new(StatusCode::BAD_REQUEST, "unknown sender"))?;
    let store = Arc::clone(&app.store);
    let joined = tokio::task::spawn_blocking(move || {
        store
            .read(move |conn| seat_token(conn, &seat_id).map_err(crewmux_store::Error::from))
            .map_err(crewmux_store::Error::into_chat)
    })
    .await
    .map_err(|err| Reject::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    joined.map_err(|err| chat_reject(&err))
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

#[derive(Serialize)]
struct Cleared {
    ok: bool,
    deleted: u64,
}

#[derive(Debug, Default, Deserialize)]
struct HistoryQs {
    topic: Option<String>,
    limit: Option<String>,
    after: Option<String>,
    since: Option<String>,
    sender: Option<String>,
}

fn since_bound(value: Option<&str>) -> Result<Option<crewmux_core::Timestamp>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    }
    parse_since(value, now_ts())
        .map(Some)
        .map_err(|_| rejected_since(value))
}

fn topic_filter(value: Option<&str>) -> Result<Option<TopicId>, Reject> {
    match value.filter(|text| !text.is_empty()) {
        None => Ok(None),
        Some(text) => TopicId::new(text)
            .map(Some)
            .map_err(|_| Reject::new(StatusCode::BAD_REQUEST, format!("unknown topic: {text}"))),
    }
}

fn cursor_filter(value: Option<&str>) -> Result<Option<ChatCursor>, Reject> {
    match value.filter(|text| !text.is_empty()) {
        None => Ok(None),
        Some(text) => ChatCursor::new(text)
            .map(Some)
            .map_err(|_| Reject::new(StatusCode::BAD_REQUEST, "invalid cursor")),
    }
}

fn sender_filter(value: Option<&str>) -> Result<Option<SenderRef>, Reject> {
    match value.filter(|text| !text.is_empty()) {
        None => Ok(None),
        Some(text) => SenderRef::parse(text)
            .map(Some)
            .map_err(|_| Reject::new(StatusCode::BAD_REQUEST, "invalid sender")),
    }
}

fn limit_filter(value: Option<&str>) -> Result<u32, Reject> {
    match value {
        None => Ok(100),
        Some(text) => text
            .parse::<u32>()
            .map_err(|_| Reject::new(StatusCode::BAD_REQUEST, "invalid limit")),
    }
}

fn object(bytes: &[u8]) -> Map<String, Value> {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default()
}

fn parse_rig(text: &str) -> Option<RigId> {
    RigId::from_str(text).ok()
}

struct Reject {
    status: StatusCode,
    message: String,
}

impl Reject {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn response(self) -> Response {
        json_error(self.status, self.message)
    }
}

fn chat_reject(err: &ChatError) -> Reject {
    let status = match err {
        ChatError::UnknownRig => StatusCode::NOT_FOUND,
        ChatError::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
        ChatError::UnparseableSince(_) | ChatError::UnknownTopic(_) | ChatError::UnknownSender => {
            StatusCode::BAD_REQUEST
        }
    };
    Reject::new(status, err.to_string())
}

fn chat_error(err: &ChatError) -> Response {
    chat_reject(err).response()
}

fn json_error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({"error": message.into()}))).into_response()
}

async fn write_chat<T, F>(app: &App, job: F) -> Result<T, ChatError>
where
    T: Send + 'static,
    F: for<'a> FnOnce(&'a Db<'a>) -> Result<T, ChatError> + Send + 'static,
{
    let store = Arc::clone(&app.store);
    let joined = tokio::task::spawn_blocking(move || {
        store.write(|conn| job(&Db::new(conn)).map_err(crewmux_store::Error::from))
    })
    .await
    .map_err(|err| ChatError::Store(err.to_string()))?;
    joined.map_err(crewmux_store::Error::into_chat)
}

async fn read_chat<T, F>(app: &App, job: F) -> Result<T, ChatError>
where
    T: Send + 'static,
    F: for<'a> FnOnce(&'a Db<'a>) -> Result<T, ChatError> + Send + 'static,
{
    let store = Arc::clone(&app.store);
    let joined = tokio::task::spawn_blocking(move || {
        store.read(|conn| job(&Db::new(conn)).map_err(crewmux_store::Error::from))
    })
    .await
    .map_err(|err| ChatError::Store(err.to_string()))?;
    joined.map_err(crewmux_store::Error::into_chat)
}
