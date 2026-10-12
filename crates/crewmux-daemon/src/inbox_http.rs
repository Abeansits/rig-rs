//! Inbox drop, absorb, deny, and the outbox list.

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use crewmux_messaging::{InboxEntry, OutboxRecord, SessionRef};
use crewmux_types::QItemId;
use serde::Deserialize;
use serde_json::json;

use crate::error::Error;
use crate::http::{fail, ok};
use crate::launch::now_ts;
use crate::state::App;

pub(crate) async fn drop_inbox(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let parsed: DropBody = match serde_json::from_slice(&body) {
        Ok(parsed) => parsed,
        Err(err) => return fail(Error::Message(err.to_string())),
    };
    let entry = match entry_from(parsed) {
        Ok(entry) => entry,
        Err(err) => return fail(err),
    };
    let mut mail = app.mail.lock().await;
    match mail.drop_inbox(entry) {
        Ok(row) => json_row(&row),
        Err(err) => fail(Error::Message(err.to_string())),
    }
}

pub(crate) async fn absorb(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let parsed: AbsorbBody = match serde_json::from_slice(&body) {
        Ok(parsed) => parsed,
        Err(err) => return fail(Error::Message(err.to_string())),
    };
    let qitem = match QItemId::new(parsed.qitem) {
        Ok(id) => id,
        Err(err) => return fail(Error::Message(err.to_string())),
    };
    let mut mail = app.mail.lock().await;
    match mail.absorb(&parsed.id, qitem) {
        Ok(row) => json_row(&row),
        Err(err) => fail(Error::Message(err.to_string())),
    }
}

pub(crate) async fn deny(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let parsed: DenyBody = match serde_json::from_slice(&body) {
        Ok(parsed) => parsed,
        Err(err) => return fail(Error::Message(err.to_string())),
    };
    let mut mail = app.mail.lock().await;
    match mail.deny(&parsed.id, &parsed.reason) {
        Ok(row) => json_row(&row),
        Err(err) => fail(Error::Message(err.to_string())),
    }
}

pub(crate) async fn outbox(State(app): State<App>, body: Bytes) -> Response {
    app.touch();
    let parsed: OutboxBody = match serde_json::from_slice(&body) {
        Ok(parsed) => parsed,
        Err(err) => return fail(Error::Message(err.to_string())),
    };
    let destination = match SessionRef::new(parsed.destination) {
        Ok(session) => session,
        Err(err) => return fail(Error::Message(err.to_string())),
    };
    let mail = app.mail.lock().await;
    match serde_json::to_value(mail.outbox_for(&destination)) {
        Ok(rows) => ok(json!({"outbox": rows})),
        Err(err) => fail(Error::Message(err.to_string())),
    }
}

pub(crate) async fn record(app: &App, record: OutboxRecord) {
    app.mail.lock().await.record_outbox(record);
}

fn json_row(row: &InboxEntry) -> Response {
    match serde_json::to_value(row) {
        Ok(value) => ok(value),
        Err(err) => fail(Error::Message(err.to_string())),
    }
}

fn entry_from(parsed: DropBody) -> Result<InboxEntry, Error> {
    Ok(InboxEntry {
        id: parsed.id,
        destination: SessionRef::new(parsed.destination)
            .map_err(|err| Error::Message(err.to_string()))?,
        sender: SessionRef::new(parsed.sender).map_err(|err| Error::Message(err.to_string()))?,
        body: parsed.body,
        state: crewmux_messaging::InboxState::Pending,
        dropped_at: now_ts(),
        absorbed_qitem: None,
        denied_reason: None,
    })
}

#[derive(Deserialize)]
struct DropBody {
    id: String,
    destination: String,
    sender: String,
    body: String,
}

#[derive(Deserialize)]
struct AbsorbBody {
    id: String,
    qitem: String,
}

#[derive(Deserialize)]
struct DenyBody {
    id: String,
    reason: String,
}

#[derive(Deserialize)]
struct OutboxBody {
    destination: String,
}
