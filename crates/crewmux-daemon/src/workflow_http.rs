//! `/api/workflow` routes. Status codes follow the TypeScript error map.

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use crewmux_orchestration::{
    Engine, Exit, InstantiateRequest, ProjectCtx, ProjectRequest, Runtime, deadline_for,
    guidance_lines, instance_json, instantiate_spec_json, project_json, spec_row_json, trail_json,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::launch::now_ts;
use crate::state::App;
use crate::workflow::{json_status, missing_field, mutate_pair, read_spec, rejected, world};

pub(crate) fn routes() -> Router<App> {
    Router::new()
        .route("/api/workflow/validate", post(validate))
        .route("/api/workflow/instantiate", post(instantiate))
        .route("/api/workflow/project", post(project))
        .route("/api/workflow/list", get(list))
        .route("/api/workflow/specs", get(specs))
        .route("/api/workflow/{id}/trace", get(trace))
        .route("/api/workflow/{id}/continue", post(continue_one))
        .route("/api/workflow/{id}/guidance", get(guidance))
        .route("/api/workflow/{id}/route", post(route_one))
        .route("/api/workflow/{id}/resume", post(resume_one))
        .route("/api/workflow/{id}/abort", post(abort_one))
        .route("/api/workflow/{id}", get(show))
}

async fn validate(State(app): State<App>, body: Bytes) -> axum::response::Response {
    app.touch();
    let parsed: SpecPath = match serde_json::from_slice(&body) {
        Ok(parsed) => parsed,
        Err(err) => return missing_field(&err.to_string()),
    };
    let Some(path) = parsed.spec_path else {
        return missing_field("specPath");
    };
    let text = match read_spec(&path).await {
        Ok(text) => text,
        Err(err) => return rejected(err),
    };
    match Runtime::validate_text(&text, &path) {
        Ok(report) => json_status(
            StatusCode::OK,
            crewmux_orchestration::validation_json(&report),
        ),
        Err(err) => rejected(err),
    }
}

async fn instantiate(State(app): State<App>, body: Bytes) -> axum::response::Response {
    app.touch();
    let parsed: InstantiateBody = match serde_json::from_slice(&body) {
        Ok(parsed) => parsed,
        Err(err) => return missing_field(&err.to_string()),
    };
    let Some(path) = parsed.spec_path else {
        return missing_field("specPath");
    };
    let Some(objective) = parsed.root_objective else {
        return missing_field("rootObjective");
    };
    let Some(created_by) = parsed.created_by else {
        return missing_field("createdBySession");
    };
    let text = match read_spec(&path).await {
        Ok(text) => text,
        Err(err) => return rejected(err),
    };
    let inventory = world(&app).await;
    let now = now_ts();
    let entry_owner = parsed.entry_owner.clone();
    let rig = parsed.target_rig.clone();
    let created = mutate_pair(&app, |engine, book| {
        let (engine, book, result) = Runtime::instantiate(
            engine,
            book,
            &InstantiateRequest {
                yaml: &text,
                path: &path,
                objective: &objective,
                created_by: &created_by,
                entry_owner: entry_owner.as_deref(),
                rig: rig.as_deref(),
            },
            &inventory,
            now,
        )?;
        let body = instantiate_body(&engine, &result);
        Ok((engine, book, body))
    })
    .await;
    match created {
        Ok(value) => json_status(StatusCode::CREATED, value),
        Err(response) => *response,
    }
}

async fn project(State(app): State<App>, body: Bytes) -> axum::response::Response {
    app.touch();
    let parsed: ProjectBody = match serde_json::from_slice(&body) {
        Ok(parsed) => parsed,
        Err(err) => return missing_field(&err.to_string()),
    };
    let Some(instance_id) = parsed.instance_id else {
        return missing_field("instanceId");
    };
    let Some(packet_id) = parsed.current_packet_id else {
        return missing_field("currentPacketId");
    };
    let Some(exit_text) = parsed.exit else {
        return missing_field("exit");
    };
    let Some(actor) = parsed.actor_session else {
        return missing_field("actorSession");
    };
    let Some(exit) = Exit::parse(&exit_text) else {
        return rejected(crewmux_orchestration::Error::Rejected {
            code: "exit_not_allowed",
            message: format!("exit `{exit_text}` is not allowed"),
            status: 400,
        });
    };
    let inventory = world(&app).await;
    let now = now_ts();
    let request = ProjectRequest {
        instance_id: instance_id.clone(),
        packet_id,
        exit,
        actor,
        result_note: parsed.result_note,
        blocked_on: parsed.blocked_on,
        evidence: parsed.closure_evidence,
        next_owner: parsed.next_owner_session,
        expected_version: None,
    };
    let projected = mutate_pair(&app, |engine, book| {
        let mut observe = || {};
        let mut ctx = ProjectCtx {
            world: &inventory,
            now,
            crash: None,
            observe: &mut observe,
        };
        let (engine, book, projection) = Runtime::project(engine, book, &request, &mut ctx)?;
        let instance = engine
            .instance(&request.instance_id)
            .ok_or_else(|| unknown_err(&request.instance_id))?;
        let body = project_json(instance, &projection);
        Ok((engine, book, body))
    })
    .await;
    match projected {
        Ok(value) => json_status(StatusCode::OK, value),
        Err(response) => *response,
    }
}

async fn list(
    State(app): State<App>,
    Query(query): Query<StatusQuery>,
) -> axum::response::Response {
    app.touch();
    let (engine, book) = snapshot(&app).await;
    let rows = engine
        .instances
        .iter()
        .filter(|instance| {
            query
                .status
                .as_deref()
                .is_none_or(|status| instance.status.as_str() == status)
        })
        .map(|instance| {
            let deadline = deadline_for(&engine, &book, instance.id.as_str(), now_ts());
            instance_json(instance, deadline.as_ref())
        })
        .collect::<Vec<_>>();
    json_status(StatusCode::OK, Value::Array(rows))
}

async fn specs(State(app): State<App>) -> axum::response::Response {
    app.touch();
    let engine = app.workflow.lock().await.clone();
    let mut rows = Vec::new();
    for cached in engine.specs.values() {
        if let Ok(spec) = crewmux_spec::parse_workflow(&cached.yaml, &cached.source_path) {
            rows.push(spec_row_json(cached, &spec));
        }
    }
    json_status(StatusCode::OK, json!({"specs": rows}))
}

async fn show(State(app): State<App>, Path(id): Path<String>) -> axum::response::Response {
    app.touch();
    let (engine, book) = snapshot(&app).await;
    let Some(instance) = engine.instance(&id) else {
        return unknown(&id);
    };
    let deadline = deadline_for(&engine, &book, &id, now_ts());
    let mut body = instance_json(instance, deadline.as_ref());
    if let Value::Object(map) = &mut body {
        map.insert(
            "guidance".to_owned(),
            json!({"lines": guidance_lines(&engine, &id)}),
        );
        map.insert("frontierPackets".to_owned(), json!([]));
        map.insert("failureOccurrences".to_owned(), json!([]));
        map.insert("unknowns".to_owned(), json!([]));
        map.insert("boundaryObligations".to_owned(), json!([]));
    }
    json_status(StatusCode::OK, body)
}

async fn trace(State(app): State<App>, Path(id): Path<String>) -> axum::response::Response {
    app.touch();
    read_view(&app, &id).await
}

async fn continue_one(State(app): State<App>, Path(id): Path<String>) -> axum::response::Response {
    app.touch();
    let (engine, book) = snapshot(&app).await;
    let before = book.epoch();
    if let Err(err) = Runtime::continue_view(&engine, &id) {
        return rejected(err);
    }
    if Runtime::continue_view(&engine, &id).is_err() || book.epoch() != before {
        return unknown(&id);
    }
    read_view(&app, &id).await
}

async fn guidance(State(app): State<App>, Path(id): Path<String>) -> axum::response::Response {
    app.touch();
    let engine = app.workflow.lock().await.clone();
    if engine.instance(&id).is_none() {
        return unknown(&id);
    }
    json_status(
        StatusCode::OK,
        json!({"lines": guidance_lines(&engine, &id)}),
    )
}

async fn route_one(
    State(app): State<App>,
    Path(id): Path<String>,
    body: Bytes,
) -> axum::response::Response {
    app.touch();
    let parsed: RouteBody = match serde_json::from_slice(&body) {
        Ok(parsed) => parsed,
        Err(err) => return missing_field(&err.to_string()),
    };
    let Some(to) = parsed.to_session else {
        return missing_field("toSession");
    };
    let Some(actor) = parsed.actor_session else {
        return missing_field("actorSession");
    };
    let now = now_ts();
    let routed = mutate_pair(&app, |engine, book| {
        let instance = engine.instance(&id).ok_or_else(|| unknown_err(&id))?;
        let step = instance.current_step_id.clone();
        let from = instance
            .frontier
            .first()
            .and_then(|packet| book.get(packet))
            .map(|item| item.destination.as_str().to_owned());
        let (engine, book, packet) = Runtime::route(engine, book, &id, &to, &actor, now)?;
        Ok((
            engine,
            book,
            json!({
                "stepId": step,
                "closedPacketId": packet,
                "newPacketId": packet,
                "fromSession": from,
                "toSession": to,
                "instanceStatus": "active",
            }),
        ))
    })
    .await;
    match routed {
        Ok(value) => json_status(StatusCode::OK, value),
        Err(response) => *response,
    }
}

async fn resume_one(
    State(app): State<App>,
    Path(id): Path<String>,
    body: Bytes,
) -> axum::response::Response {
    app.touch();
    let parsed: ResumeBody = match serde_json::from_slice(&body) {
        Ok(parsed) => parsed,
        Err(err) => return missing_field(&err.to_string()),
    };
    let Some(actor) = parsed.actor_session else {
        return missing_field("actorSession");
    };
    let now = now_ts();
    let resumed = mutate_pair(&app, |engine, book| {
        let instance = engine.instance(&id).ok_or_else(|| unknown_err(&id))?;
        let owner = instance.created_by.as_str().to_owned();
        let (engine, book, packet) = Runtime::resume(engine, book, &id, &actor, &owner, now)?;
        let row = engine.instance(&id).ok_or_else(|| unknown_err(&id))?;
        let step = row.current_step_id.clone();
        let resume_count = row.resume_count;
        Ok((
            engine,
            book,
            json!({
                "stepId": step,
                "newPacketId": packet,
                "ownerSession": owner,
                "resumeCount": resume_count,
                "exceptionItemsClosed": 0,
            }),
        ))
    })
    .await;
    match resumed {
        Ok(value) => json_status(StatusCode::OK, value),
        Err(response) => *response,
    }
}

async fn abort_one(
    State(app): State<App>,
    Path(id): Path<String>,
    body: Bytes,
) -> axum::response::Response {
    app.touch();
    let parsed: AbortBody = match serde_json::from_slice(&body) {
        Ok(parsed) => parsed,
        Err(err) => return missing_field(&err.to_string()),
    };
    let Some(reason) = parsed.reason.filter(|text| !text.trim().is_empty()) else {
        return missing_field("reason");
    };
    let Some(actor) = parsed.actor_session else {
        return missing_field("actorSession");
    };
    let now = now_ts();
    let aborted = mutate_pair(&app, |engine, book| {
        let closed = engine
            .instance(&id)
            .map(|instance| {
                instance
                    .frontier
                    .iter()
                    .map(|packet| packet.as_str().to_owned())
                    .collect::<Vec<_>>()
            })
            .ok_or_else(|| unknown_err(&id))?;
        let (engine, book) = Runtime::abort(engine, book, &id, &actor, &reason, now)?;
        Ok((
            engine,
            book,
            json!({"instanceId": id, "closedPacketIds": closed, "status": "aborted"}),
        ))
    })
    .await;
    match aborted {
        Ok(value) => json_status(StatusCode::OK, value),
        Err(response) => *response,
    }
}

fn instantiate_body(engine: &Engine, result: &crewmux_orchestration::Instantiate) -> Value {
    let Some(instance) = engine.instance(&result.instance_id) else {
        return json!({"instanceId": result.instance_id});
    };
    let spec = engine
        .specs
        .get(&Engine::spec_key(
            &instance.workflow_name,
            &instance.workflow_version,
        ))
        .and_then(|cached| {
            crewmux_spec::parse_workflow(&cached.yaml, &cached.source_path)
                .ok()
                .map(|spec| instantiate_spec_json(cached, &spec))
        });
    json!({
        "instance": instance_json(instance, None),
        "spec": spec,
        "entryQitemId": result.entry_qitem,
        "entryOwnerSession": result.entry_owner,
        "advisories": result.advisories,
        "replayed": result.replayed,
    })
}

async fn read_view(app: &App, id: &str) -> axum::response::Response {
    let (engine, book) = snapshot(app).await;
    let Some(instance) = engine.instance(id) else {
        return unknown(id);
    };
    let deadline = deadline_for(&engine, &book, id, now_ts());
    let trail = engine
        .trails
        .iter()
        .filter(|row| row.instance_id == id)
        .map(trail_json)
        .collect::<Vec<_>>();
    json_status(
        StatusCode::OK,
        json!({
            "instance": instance_json(instance, deadline.as_ref()),
            "trail": trail,
            "guidance": {"lines": guidance_lines(&engine, id)},
            "frontier": [],
            "failures": [],
            "unknowns": [],
            "boundaryObligations": [],
        }),
    )
}

async fn snapshot(app: &App) -> (Engine, crewmux_messaging::QueueBook) {
    let engine = app.workflow.lock().await.clone();
    let book = app.queue.lock().await.clone();
    (engine, book)
}

fn unknown(id: &str) -> axum::response::Response {
    rejected(unknown_err(id))
}

fn unknown_err(id: &str) -> crewmux_orchestration::Error {
    crewmux_orchestration::Error::Rejected {
        code: "instance_not_found",
        message: format!("workflow instance {id} not found"),
        status: 404,
    }
}

#[derive(Deserialize)]
struct SpecPath {
    #[serde(rename = "specPath")]
    spec_path: Option<String>,
}

#[derive(Deserialize)]
struct InstantiateBody {
    #[serde(rename = "specPath")]
    spec_path: Option<String>,
    #[serde(rename = "rootObjective")]
    root_objective: Option<String>,
    #[serde(rename = "createdBySession")]
    created_by: Option<String>,
    #[serde(rename = "entryOwnerSession")]
    entry_owner: Option<String>,
    #[serde(rename = "targetRig")]
    target_rig: Option<String>,
}

#[derive(Deserialize)]
struct ProjectBody {
    #[serde(rename = "instanceId")]
    instance_id: Option<String>,
    #[serde(rename = "currentPacketId")]
    current_packet_id: Option<String>,
    exit: Option<String>,
    #[serde(rename = "actorSession")]
    actor_session: Option<String>,
    #[serde(rename = "resultNote")]
    result_note: Option<String>,
    #[serde(rename = "blockedOn")]
    blocked_on: Option<String>,
    #[serde(rename = "closureEvidence")]
    closure_evidence: Option<serde_json::Value>,
    #[serde(rename = "nextOwnerSession")]
    next_owner_session: Option<String>,
}

#[derive(Deserialize)]
struct StatusQuery {
    status: Option<String>,
}

#[derive(Deserialize)]
struct RouteBody {
    #[serde(rename = "toSession")]
    to_session: Option<String>,
    #[serde(rename = "actorSession")]
    actor_session: Option<String>,
}

#[derive(Deserialize)]
struct ResumeBody {
    #[serde(rename = "actorSession")]
    actor_session: Option<String>,
}

#[derive(Deserialize)]
struct AbortBody {
    reason: Option<String>,
    #[serde(rename = "actorSession")]
    actor_session: Option<String>,
}
