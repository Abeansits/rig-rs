// Derived from openrig packages/daemon/src/routes/stream.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Stream emit, list, archive, classify, and the classifier lease.

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use crewmux_api::{StreamKind, StreamPayload};
use crewmux_health::{
    CandidateSet, ClassificationDecision, ClassifierLease, DEFAULT_LEASE_TTL_MS, LeaseState,
    StreamItem, cap_page, iso_millis, validate_decision,
};
use crewmux_store::LeaseClock;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::Error;
use crate::events;
use crate::health_http::reply;
use crate::health_query::{now_i64, write_db};
use crate::state::App;

pub(crate) fn routes() -> Router<App> {
    Router::new()
        .route("/api/stream/emit", post(emit))
        .route("/api/stream/list", get(list))
        .route("/api/stream/watch", get(events::stream_watch))
        .route("/api/stream/sse", get(events::stream_watch))
        .route("/api/stream/lease/acquire", post(acquire))
        .route("/api/stream/lease/heartbeat", post(heartbeat))
        .route("/api/stream/lease/evaluate", post(evaluate))
        .route("/api/stream/lease/reclaim", post(reclaim))
        .route("/api/stream/reclaim-classifier", post(reclaim))
        .route("/api/stream/classify", post(classify))
        .route("/api/stream/{id}", get(show))
        .route("/api/stream/{id}/archive", post(archive))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EmitBody {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    stream_item_id: Option<String>,
    body: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    source_session: String,
}

async fn emit(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let parsed = match serde_json::from_slice::<EmitBody>(&body) {
        Ok(parsed) => parsed,
        Err(err) => return reply(Err(rejected(err.to_string()))),
    };
    let id = parsed.id.or(parsed.stream_item_id);
    let source = if parsed.source.is_empty() {
        parsed.source_session
    } else {
        parsed.source
    };
    let kind = parsed.kind;
    let text = parsed.body;
    let at_ms = now_i64();
    let emitted = write_db(&app, move |conn| {
        crewmux_store::stream_emit(
            conn,
            &crewmux_store::NewStream {
                id,
                kind,
                body: text,
                source,
                at_ms,
            },
        )
    })
    .await;
    match emitted {
        Ok(emitted) => {
            if emitted.created {
                publish_emit(&app, &emitted.item).await;
            }
            let status = if emitted.created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            };
            (status, Json(item_json(&emitted.item))).into_response()
        }
        Err(err) => reply(Err(err)),
    }
}

async fn publish_emit(app: &App, item: &StreamItem) {
    let summary: String = item.body.chars().take(80).collect();
    app.hub
        .publish(
            StreamKind::StreamEmitted,
            Some(StreamPayload {
                summary: Some(summary),
                stream_id: Some(item.id.clone()),
                seq: u64::try_from(item.seq).ok(),
                ..StreamPayload::default()
            }),
        )
        .await;
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    limit: Option<u32>,
    #[serde(default)]
    include_archived: bool,
}

async fn list(State(app): State<App>, Query(query): Query<ListQuery>) -> Response {
    app.touch();
    let limit = query.limit.unwrap_or(50);
    reply(
        write_db(&app, move |conn| {
            let items = crewmux_store::stream_list(conn, limit, query.include_archived)?;
            Ok(json!(items.iter().map(item_json).collect::<Vec<_>>()))
        })
        .await,
    )
}

async fn show(State(app): State<App>, Path(id): Path<String>) -> Response {
    app.touch();
    match write_db(&app, move |conn| crewmux_store::stream_get(conn, &id)).await {
        Ok(Some(item)) => Json(item_json(&item)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "stream item not found"})),
        )
            .into_response(),
        Err(err) => reply(Err(err)),
    }
}

async fn archive(State(app): State<App>, Path(id): Path<String>, body: Bytes) -> Response {
    // Read the body before the update. Replying first lets the peer see a
    // reset while it is still writing, and a retry then archives twice.
    drop(body);
    app.touch();
    reply(
        write_db(&app, move |conn| {
            let item = crewmux_store::stream_archive(conn, &id)?;
            Ok(json!({"ok": true, "item": item_json(&item)}))
        })
        .await,
    )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AcquireBody {
    classifier_session: String,
    #[serde(default)]
    evaluate_deadness_first: bool,
}

async fn acquire(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let parsed = match serde_json::from_slice::<AcquireBody>(&body) {
        Ok(parsed) => parsed,
        Err(err) => return reply(Err(rejected(err.to_string()))),
    };
    let now = now_i64();
    reply(
        write_db(&app, move |conn| {
            if parsed.evaluate_deadness_first {
                crewmux_store::lease_evaluate(conn, now)?;
            }
            let lease = crewmux_store::lease_acquire(conn, &parsed.classifier_session, clock(now))?;
            Ok(lease_json(&lease))
        })
        .await,
    )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HeartbeatBody {
    lease_id: String,
    classifier_session: String,
}

async fn heartbeat(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let parsed = match serde_json::from_slice::<HeartbeatBody>(&body) {
        Ok(parsed) => parsed,
        Err(err) => return reply(Err(rejected(err.to_string()))),
    };
    let now = now_i64();
    reply(
        write_db(&app, move |conn| {
            let lease = crewmux_store::lease_heartbeat(
                conn,
                &parsed.lease_id,
                &parsed.classifier_session,
                clock(now),
            )?;
            Ok(lease_json(&lease))
        })
        .await,
    )
}

async fn evaluate(State(app): State<App>) -> Response {
    app.touch();
    let now = now_i64();
    reply(
        write_db(&app, move |conn| {
            let expired = crewmux_store::lease_evaluate(conn, now)?;
            Ok(json!({"expired": expired}))
        })
        .await,
    )
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ReclaimBody {
    #[serde(default)]
    if_dead: bool,
}

async fn reclaim(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let parsed = if body.is_empty() {
        ReclaimBody::default()
    } else {
        match serde_json::from_slice::<ReclaimBody>(&body) {
            Ok(parsed) => parsed,
            Err(err) => return reply(Err(rejected(err.to_string()))),
        }
    };
    let now = now_i64();
    reply(
        write_db(&app, move |conn| {
            let lease = crewmux_store::lease_reclaim(conn, parsed.if_dead, now)?;
            Ok(lease_json(&lease))
        })
        .await,
    )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClassifyBody {
    stream_item_id: String,
    classifier_session: String,
    lease_id: String,
    candidates: CandidateSet,
    decision: ClassificationDecision,
    #[serde(default)]
    limit: Option<u32>,
}

async fn classify(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let parsed = match serde_json::from_slice::<ClassifyBody>(&body) {
        Ok(parsed) => parsed,
        Err(err) => return reply(Err(rejected(err.to_string()))),
    };
    let now = now_i64();
    reply(write_db(&app, move |conn| record_decision(conn, &parsed, now)).await)
}

fn record_decision(
    conn: &crewmux_store::Connection,
    body: &ClassifyBody,
    now: i64,
) -> Result<Value, crewmux_store::Error> {
    if let Some(limit) = body.limit {
        cap_page(limit)?;
    }
    validate_decision(&body.candidates, &body.decision)?;
    let lease = crewmux_store::lease_active(conn)?.ok_or(crewmux_health::Error::NoActiveLease)?;
    hold(&lease, &body.classifier_session, &body.lease_id, now)?;
    let decision = serde_json::to_string(&body.decision).map_err(|err| {
        crewmux_store::Error::Health(crewmux_health::Error::Classify(err.to_string()))
    })?;
    crewmux_store::attempt_insert(
        conn,
        &body.stream_item_id,
        &body.lease_id,
        &body.classifier_session,
        &decision,
        now,
    )?;
    Ok(json!({
        "recorded": true,
        "streamItemId": body.stream_item_id,
        "decision": body.decision,
    }))
}

fn hold(
    lease: &ClassifierLease,
    session: &str,
    lease_id: &str,
    now: i64,
) -> Result<(), crewmux_store::Error> {
    if lease.state != LeaseState::Active || now > lease.expires_at_ms {
        return Err(crewmux_health::Error::LeaseNotActive.into());
    }
    if lease.id != lease_id {
        return Err(crewmux_health::Error::LeaseMismatch.into());
    }
    if lease.holder.as_str() != session {
        return Err(crewmux_health::Error::LeaseHeld {
            holder: lease.holder.as_str().to_owned(),
        }
        .into());
    }
    Ok(())
}

fn clock(now_ms: i64) -> LeaseClock {
    LeaseClock {
        now_ms,
        ttl_ms: DEFAULT_LEASE_TTL_MS,
    }
}

fn item_json(item: &StreamItem) -> Value {
    json!({
        "id": item.id,
        "streamItemId": item.id,
        "kind": item.kind,
        "body": item.body,
        "labels": item.labels,
        "archived": item.archived,
        "source": item.source,
        "sourceSession": item.source,
        "seq": item.seq,
        "emittedAt": iso_millis(item.emitted_at_ms),
    })
}

fn lease_json(lease: &ClassifierLease) -> Value {
    json!({
        "id": lease.id,
        "leaseId": lease.id,
        "holder": lease.holder.as_str(),
        "classifierSession": lease.holder.as_str(),
        "acquiredAt": iso_millis(lease.acquired_at_ms),
        "expiresAt": iso_millis(lease.expires_at_ms),
        "lastHeartbeat": iso_millis(lease.last_heartbeat_ms),
        "state": lease.state,
    })
}

fn rejected(message: String) -> Error {
    Error::Store(crewmux_store::Error::Health(
        crewmux_health::Error::Rejected(message),
    ))
}
