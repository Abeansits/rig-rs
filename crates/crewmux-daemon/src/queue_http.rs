//! Queue routes. Mutations commit in the book, then the writer persists the snapshot.

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use crewmux_core::DomainEvent;
use crewmux_messaging::{Actor, NewItem, QueueState, SessionRef, Update};
use crewmux_types::QItemId;
use serde::Deserialize;
use serde_json::json;

use crate::error::Error;
use crate::http::{fail, ok};
use crate::launch::now_ts;
use crate::notify::emit;
use crate::state::App;

pub(crate) async fn list(State(app): State<App>) -> Response {
    app.touch();
    let book = app.queue.lock().await;
    match serde_json::to_value(book.items()) {
        Ok(items) => ok(json!({"items": items})),
        Err(err) => fail(Error::Message(err.to_string())),
    }
}

pub(crate) async fn show(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let id = match id_of(&body) {
        Ok(id) => id,
        Err(err) => return fail(err),
    };
    let book = app.queue.lock().await;
    let Some(item) = book.get(&id) else {
        return fail(Error::Message(format!("qitem {id} not found")));
    };
    let transitions = book.transitions(&id);
    match (
        serde_json::to_value(item),
        serde_json::to_value(&transitions),
    ) {
        (Ok(item), Ok(transitions)) => ok(json!({"item": item, "transitions": transitions})),
        (Err(err), _) | (_, Err(err)) => fail(Error::Message(err.to_string())),
    }
}

pub(crate) async fn create(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let parsed: CreateBody = match serde_json::from_slice(&body) {
        Ok(parsed) => parsed,
        Err(err) => return fail(Error::Message(err.to_string())),
    };
    let now = now_ts();
    let created = mutate(&app, move |book| {
        let source = SessionRef::new(parsed.source)?;
        let destination = SessionRef::new(parsed.destination)?;
        let id = match parsed.id {
            Some(text) => {
                Some(
                    QItemId::new(text).map_err(|err| crewmux_messaging::Error::Rejected {
                        code: "invalid_id",
                        message: err.to_string(),
                    })?,
                )
            }
            None => None,
        };
        book.create(
            NewItem {
                id,
                source,
                destination,
                body: parsed.body,
                priority: parsed.priority,
                tier: parsed.tier,
                tags: Vec::new(),
                handed_off_from: None,
                last_nudge: None,
                explicit_human: parsed.explicit_human,
                fingerprint: String::new(),
            },
            now,
        )
    })
    .await;
    finish_item(&app, created).await
}

pub(crate) async fn claim(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let parsed: ClaimBody = match serde_json::from_slice(&body) {
        Ok(parsed) => parsed,
        Err(err) => return fail(Error::Message(err.to_string())),
    };
    let now = now_ts();
    let claimed = mutate(&app, move |book| {
        let id = qid(&parsed.id)?;
        let by = SessionRef::new(parsed.by)?;
        book.claim(&id, &by, now, parsed.force.as_deref())
    })
    .await;
    finish_item(&app, claimed).await
}

pub(crate) async fn unclaim(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let parsed: UnclaimBody = match serde_json::from_slice(&body) {
        Ok(parsed) => parsed,
        Err(err) => return fail(Error::Message(err.to_string())),
    };
    let now = now_ts();
    let item = mutate(&app, move |book| {
        let id = qid(&parsed.id)?;
        let by = SessionRef::new(parsed.by)?;
        book.unclaim(&id, &by, &parsed.reason, now)
    })
    .await;
    finish_item(&app, item).await
}

pub(crate) async fn handoff(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    handoff_with(&app, &body, false).await
}

pub(crate) async fn handoff_complete(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    handoff_with(&app, &body, true).await
}

pub(crate) async fn fallback(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let parsed: HandoffBody = match serde_json::from_slice(&body) {
        Ok(parsed) => parsed,
        Err(err) => return fail(Error::Message(err.to_string())),
    };
    let now = now_ts();
    let item = mutate(&app, move |book| {
        let id = qid(&parsed.id)?;
        let to = SessionRef::new(parsed.to)?;
        let actor = SessionRef::new(parsed.actor)?;
        book.fallback(&id, &to, &actor, now)
    })
    .await;
    finish_item(&app, item).await
}

pub(crate) async fn update_item(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let parsed: UpdateBody = match serde_json::from_slice(&body) {
        Ok(parsed) => parsed,
        Err(err) => return fail(Error::Message(err.to_string())),
    };
    let now = now_ts();
    let transition = mutate(&app, move |book| {
        let id = qid(&parsed.id)?;
        let actor = SessionRef::new(parsed.actor)?;
        let state = match parsed.state {
            Some(text) => Some(QueueState::parse(&text).ok_or_else(|| {
                crewmux_messaging::Error::Rejected {
                    code: "bad_state",
                    message: format!("unknown state {text}"),
                }
            })?),
            None => None,
        };
        let blocked_on = match parsed.blocked_on {
            Some(text) => Some(SessionRef::new(text)?),
            None => None,
        };
        book.update(
            &id,
            Update {
                state,
                actor: Actor::Session(actor),
                note: parsed.note,
                closure_reason: parsed.closure_reason,
                closure_target: parsed.closure_target,
                reopen: parsed.reopen,
                blocked_on,
            },
            now,
        )
    })
    .await;
    match transition {
        Ok(transition) => {
            emit(
                &app,
                DomainEvent::QueueChanged {
                    qitem: transition.qitem.clone(),
                    state: transition.to.as_str().to_owned(),
                    at: now_ts(),
                },
            )
            .await;
            match serde_json::to_value(&transition) {
                Ok(value) => ok(json!({"transition": value})),
                Err(err) => fail(Error::Message(err.to_string())),
            }
        }
        Err(err) => fail(err),
    }
}

async fn handoff_with(app: &App, body: &[u8], complete: bool) -> Response {
    let parsed: HandoffBody = match serde_json::from_slice(body) {
        Ok(parsed) => parsed,
        Err(err) => return fail(Error::Message(err.to_string())),
    };
    let now = now_ts();
    let pair = mutate(app, move |book| {
        let id = qid(&parsed.id)?;
        let to = SessionRef::new(parsed.to)?;
        let actor = SessionRef::new(parsed.actor)?;
        book.handoff(&id, &to, &actor, now, complete)
    })
    .await;
    match pair {
        Ok((closed, successor)) => {
            emit(
                app,
                DomainEvent::QueueChanged {
                    qitem: successor.id.clone(),
                    state: successor.state.as_str().to_owned(),
                    at: now_ts(),
                },
            )
            .await;
            match (
                serde_json::to_value(&closed),
                serde_json::to_value(&successor),
            ) {
                (Ok(closed), Ok(successor)) => {
                    ok(json!({"closed": closed, "successor": successor}))
                }
                (Err(err), _) | (_, Err(err)) => fail(Error::Message(err.to_string())),
            }
        }
        Err(err) => fail(err),
    }
}

async fn finish_item(app: &App, item: Result<crewmux_messaging::QueueItem, Error>) -> Response {
    match item {
        Ok(item) => {
            emit(
                app,
                DomainEvent::QueueChanged {
                    qitem: item.id.clone(),
                    state: item.state.as_str().to_owned(),
                    at: now_ts(),
                },
            )
            .await;
            match serde_json::to_value(&item) {
                Ok(value) => ok(json!({"item": value})),
                Err(err) => fail(Error::Message(err.to_string())),
            }
        }
        Err(err) => fail(err),
    }
}

async fn mutate<T>(
    app: &App,
    f: impl FnOnce(&mut crewmux_messaging::QueueBook) -> Result<T, crewmux_messaging::Error>,
) -> Result<T, Error> {
    let mut book = app.queue.lock().await;
    let value = f(&mut book).map_err(|err| Error::Message(err.to_string()))?;
    let snap = book.clone();
    drop(book);
    app.submit_queue(snap);
    Ok(value)
}

fn id_of(body: &[u8]) -> Result<QItemId, Error> {
    let parsed: IdBody =
        serde_json::from_slice(body).map_err(|err| Error::Message(err.to_string()))?;
    qid(&parsed.id).map_err(|err| Error::Message(err.to_string()))
}

fn qid(text: &str) -> Result<QItemId, crewmux_messaging::Error> {
    QItemId::new(text).map_err(|err| crewmux_messaging::Error::Rejected {
        code: "invalid_id",
        message: err.to_string(),
    })
}

#[derive(Deserialize)]
struct IdBody {
    id: String,
}

#[derive(Deserialize)]
struct CreateBody {
    #[serde(default)]
    id: Option<String>,
    source: String,
    destination: String,
    body: String,
    #[serde(default)]
    tier: Option<String>,
    #[serde(default)]
    priority: i64,
    #[serde(default)]
    explicit_human: bool,
}

#[derive(Deserialize)]
struct ClaimBody {
    id: String,
    by: String,
    #[serde(default)]
    force: Option<String>,
}

#[derive(Deserialize)]
struct UnclaimBody {
    id: String,
    by: String,
    reason: String,
}

#[derive(Deserialize)]
struct HandoffBody {
    id: String,
    to: String,
    actor: String,
}

#[derive(Deserialize)]
struct UpdateBody {
    id: String,
    actor: String,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    note: String,
    #[serde(default)]
    closure_reason: Option<String>,
    #[serde(default)]
    closure_target: Option<String>,
    #[serde(default)]
    reopen: bool,
    #[serde(default)]
    blocked_on: Option<String>,
}
