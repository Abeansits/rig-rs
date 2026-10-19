// Derived from openrig packages/daemon/src/routes/health.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! `/api/health`, diagnosis, summary, and usage routes.

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use crewmux_health::{Severity, Status, parse_policy};
use crewmux_store::UsageRow;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::Error;
use crate::health_query::{contexts, nodes, now_i64, projection, read_db, write_db};
use crate::state::App;

const ABSENCE: &str = "Authored outcome-boundary censuses; absence is not evidence of health.";

pub(crate) fn routes() -> Router<App> {
    Router::new()
        .route("/api/health", get(list_health))
        .route("/api/health/{id}", get(one_health))
        .route(
            "/api/health-diagnosis/policy",
            get(policy_get).post(policy_post),
        )
        .route("/api/health-diagnosis/checkpoints", get(checkpoints))
        .route("/api/health-diagnosis", get(diagnoses))
        .route("/api/health-diagnosis/{id}", get(one_diagnosis))
        .route("/api/health-summary/nodes", get(summary_nodes))
        .route("/api/health-summary/context", get(summary_context))
        .route("/api/health-summary/version", get(version))
        .route("/api/health-summary/gateway", get(gateway))
        .route("/api/usage", get(usage))
        .route("/api/usage/sample", post(sample))
}

#[derive(Debug, Deserialize, Default)]
struct ListFilter {
    limit: Option<u32>,
    severity: Option<String>,
    status: Option<String>,
}

async fn list_health(State(app): State<App>, Query(filter): Query<ListFilter>) -> Response {
    app.touch();
    reply(list_value(&app, filter).await)
}

async fn one_health(State(app): State<App>, Path(id): Path<String>) -> Response {
    app.touch();
    match projection(&app, 200, None, None).await {
        Ok(found) => match found.records.into_iter().find(|record| record.id == id) {
            Some(record) => Json(record).into_response(),
            None => missing("health_finding_not_found"),
        },
        Err(err) => reply(Err(err)),
    }
}

async fn list_value(app: &App, filter: ListFilter) -> Result<Value, Error> {
    let limit = filter.limit.unwrap_or(100);
    let severity = match filter.severity.as_deref() {
        None => None,
        Some(text) => Some(parse_severity(text)?),
    };
    let status = match filter.status.as_deref() {
        None => None,
        Some(text) => Some(parse_status(text)?),
    };
    let projected = projection(app, limit, severity, status).await?;
    serde_json::to_value(projected).map_err(|err| Error::Message(err.to_string()))
}

async fn policy_get(State(app): State<App>) -> Response {
    app.touch();
    reply(
        read_db(&app, crewmux_store::policy_document)
            .await
            .and_then(policy_value),
    )
}

async fn policy_post(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    reply(store_policy(&app, &body).await)
}

async fn store_policy(app: &App, body: &[u8]) -> Result<Value, Error> {
    let value = serde_json::from_slice::<Value>(body).map_err(|err| {
        Error::Store(crewmux_store::Error::Health(crewmux_health::Error::Policy(
            err.to_string(),
        )))
    })?;
    parse_policy(&value).map_err(|err| Error::Store(crewmux_store::Error::Health(err)))?;
    let text = serde_json::to_string(&value).map_err(|err| Error::Message(err.to_string()))?;
    write_db(app, move |conn| crewmux_store::policy_store(conn, &text)).await?;
    Ok(value)
}

fn policy_value(document: Option<String>) -> Result<Value, Error> {
    let Some(document) = document else {
        return Ok(default_policy());
    };
    serde_json::from_str(&document).map_err(|err| {
        Error::Store(crewmux_store::Error::Health(crewmux_health::Error::Policy(
            err.to_string(),
        )))
    })
}

fn default_policy() -> Value {
    json!({
        "schema": "openrig.health-policy/v0alpha1",
        "disabledDetectors": [],
        "thresholds": {
            "ceremonyTransitions": 20,
            "ceremonyRatio": 12,
            "reviewReturns": 4,
            "redundantWakes": 4
        },
        "contextPressure": {"warningPercent": 95, "criticalPercent": 99}
    })
}

async fn checkpoints(State(app): State<App>) -> Response {
    app.touch();
    reply(checkpoint_census(&app).await)
}

async fn diagnoses(State(app): State<App>) -> Response {
    app.touch();
    reply(diagnosis_census(&app).await)
}

async fn checkpoint_census(app: &App) -> Result<Value, Error> {
    let rows = read_db(app, crewmux_store::checkpoint_list).await?;
    Ok(census("openrig.health-checkpoint/v0alpha1", rows))
}

async fn diagnosis_census(app: &App) -> Result<Value, Error> {
    let rows = read_db(app, crewmux_store::diagnosis_list).await?;
    Ok(census("openrig.health-diagnosis/v0alpha1", rows))
}

fn census(schema: &str, rows: Vec<(String, String)>) -> Value {
    json!({
        "schema": schema,
        "note": ABSENCE,
        "records": rows.into_iter().map(|(id, body)| json!({"id": id, "body": body})).collect::<Vec<_>>()
    })
}

async fn one_diagnosis(State(app): State<App>, Path(id): Path<String>) -> Response {
    app.touch();
    match read_db(&app, move |conn| crewmux_store::diagnosis_get(conn, &id)).await {
        Ok(Some(body)) => {
            Json(json!({"schema": "openrig.health-diagnosis/v0alpha1", "body": body}))
                .into_response()
        }
        Ok(None) => missing("health_diagnosis_not_found"),
        Err(err) => reply(Err(err)),
    }
}

async fn summary_nodes(State(app): State<App>) -> Response {
    app.touch();
    reply(nodes(&app).await.and_then(json_of))
}

async fn summary_context(State(app): State<App>) -> Response {
    app.touch();
    reply(contexts(&app).await.and_then(json_of))
}

async fn version(State(app): State<App>) -> Json<Value> {
    app.touch();
    Json(json!({"version": env!("CARGO_PKG_VERSION")}))
}

async fn gateway(State(app): State<App>) -> Response {
    app.touch();
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"error": "gateway_subsystem_unavailable"})),
    )
        .into_response()
}

async fn usage(State(app): State<App>) -> Response {
    app.touch();
    reply(
        read_db(&app, crewmux_store::usage_list)
            .await
            .map(|rows| json!({"samples": rows.iter().map(usage_json).collect::<Vec<_>>()})),
    )
}

async fn sample(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    reply(store_sample(&app, &body).await)
}

async fn store_sample(app: &App, body: &[u8]) -> Result<Value, Error> {
    let parsed: SampleBody = serde_json::from_slice(body).map_err(|err| {
        Error::Store(crewmux_store::Error::Health(
            crewmux_health::Error::Rejected(err.to_string()),
        ))
    })?;
    if !seat_exists(app, &parsed.seat).await? {
        return Err(rejected("seat not found"));
    }
    let row = UsageRow {
        seat: parsed.seat,
        used_tokens: parsed.used_tokens.map(i64::from),
        token_limit: parsed.token_limit.map(i64::from),
        sampled_at_ms: now_i64(),
    };
    write_db(app, {
        let row = row.clone();
        move |conn| crewmux_store::usage_upsert(conn, &row)
    })
    .await?;
    Ok(usage_json(&row))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SampleBody {
    seat: String,
    used_tokens: Option<u32>,
    token_limit: Option<u32>,
}

async fn seat_exists(app: &App, seat: &str) -> Result<bool, Error> {
    let seats = crate::db::read(&app.store, |conn| crewmux_core::SeatRepo::list_all(conn)).await?;
    for seat_row in &seats {
        if seat_row.name.as_str() == seat {
            return Ok(true);
        }
        if crate::query::address_of(app, seat_row).await? == seat {
            return Ok(true);
        }
    }
    Ok(false)
}

fn usage_json(row: &UsageRow) -> Value {
    json!({
        "seat": row.seat,
        "usedTokens": row.used_tokens,
        "tokenLimit": row.token_limit,
        "sampledAt": crewmux_health::iso_millis(row.sampled_at_ms),
    })
}

fn json_of(value: impl serde::Serialize) -> Result<Value, Error> {
    serde_json::to_value(value).map_err(|err| Error::Message(err.to_string()))
}

fn parse_severity(text: &str) -> Result<Severity, Error> {
    match text {
        "info" => Ok(Severity::Info),
        "warning" => Ok(Severity::Warning),
        "critical" => Ok(Severity::Critical),
        _ => Err(rejected("severity must be info, warning, or critical")),
    }
}

fn parse_status(text: &str) -> Result<Status, Error> {
    match text {
        "active" => Ok(Status::Active),
        "cleared" => Ok(Status::Cleared),
        "indeterminate" => Ok(Status::Indeterminate),
        _ => Err(rejected("status must be active, cleared, or indeterminate")),
    }
}

fn rejected(message: &str) -> Error {
    Error::Store(crewmux_store::Error::Health(
        crewmux_health::Error::Rejected(message.to_owned()),
    ))
}

pub(crate) fn reply(result: Result<Value, Error>) -> Response {
    match result {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(Error::Store(crewmux_store::Error::Health(err))) => health_error(&err),
        Err(err) => crate::http::fail(err),
    }
}

fn health_error(err: &crewmux_health::Error) -> Response {
    let status = match err {
        crewmux_health::Error::LeaseHeld { .. } | crewmux_health::Error::LeaseStillActive => {
            StatusCode::CONFLICT
        }
        crewmux_health::Error::LeaseMismatch => StatusCode::FORBIDDEN,
        crewmux_health::Error::LeaseNotFound | crewmux_health::Error::NoActiveLease => {
            StatusCode::NOT_FOUND
        }
        crewmux_health::Error::Rejected(message) if message.contains("not found") => {
            StatusCode::NOT_FOUND
        }
        crewmux_health::Error::Policy(_)
        | crewmux_health::Error::Rejected(_)
        | crewmux_health::Error::Limit
        | crewmux_health::Error::LeaseNotActive
        | crewmux_health::Error::LeaseExpired
        | crewmux_health::Error::Classify(_)
        | crewmux_health::Error::Hook(_)
        | crewmux_health::Error::Message(_) => StatusCode::BAD_REQUEST,
    };
    (status, Json(json!({"error": err.to_string()}))).into_response()
}

fn missing(error: &str) -> Response {
    (StatusCode::NOT_FOUND, Json(json!({"error": error}))).into_response()
}
