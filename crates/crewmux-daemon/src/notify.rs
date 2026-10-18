//! Publish a domain event to the stream and the SQLite log.

use std::sync::Arc;

use crewmux_api::{StreamKind, StreamPayload};
use crewmux_core::{DomainEvent, EventLog};
use crewmux_store::Db;

use crate::state::App;

pub(crate) async fn emit(app: &App, event: DomainEvent) {
    let (kind, payload) = project(&event);
    app.hub.publish(kind, payload).await;
    let store = Arc::clone(&app.store);
    let logged = tokio::task::spawn_blocking(move || {
        store.write(move |conn| EventLog::append(&Db::new(conn), &event).map_err(Into::into))
    })
    .await;
    if let Err(err) = logged {
        tracing::warn!(error = %err, "event log");
    }
}

fn project(event: &DomainEvent) -> (StreamKind, Option<StreamPayload>) {
    match event {
        DomainEvent::QueueChanged { qitem, state, .. } => (
            StreamKind::QueueChanged,
            Some(StreamPayload {
                qitem: Some(qitem.to_string()),
                state: Some(state.clone()),
                ..StreamPayload::default()
            }),
        ),
        DomainEvent::DeliveryRecorded { seat, outcome, .. } => (
            StreamKind::DeliveryRecorded,
            Some(StreamPayload {
                seat: Some(seat.clone()),
                outcome: Some(outcome.clone()),
                ..StreamPayload::default()
            }),
        ),
        DomainEvent::AttentionRaised { seat, reason, .. } => (
            StreamKind::AttentionRaised,
            Some(StreamPayload {
                seat: Some(seat.clone()),
                reason: Some(reason.clone()),
                ..StreamPayload::default()
            }),
        ),
        DomainEvent::SeatDied {
            seat,
            exit_status,
            capture,
            ..
        } => (
            StreamKind::SeatDied,
            Some(StreamPayload {
                seat: Some(seat.to_string()),
                exit_status: *exit_status,
                capture: capture.clone(),
                ..StreamPayload::default()
            }),
        ),
        DomainEvent::SeatReady { seat, .. }
        | DomainEvent::SeatLaunched { seat, .. }
        | DomainEvent::SeatStopped { seat, .. } => (
            StreamKind::SeatStateChanged,
            Some(StreamPayload {
                seat: Some(seat.to_string()),
                state: Some(event.kind().to_owned()),
                ..StreamPayload::default()
            }),
        ),
        DomainEvent::RigCreated { name, .. } => (
            StreamKind::Activity,
            Some(StreamPayload {
                summary: Some(name.to_string()),
                ..StreamPayload::default()
            }),
        ),
        DomainEvent::RestoreCompleted { outcome, .. } => (
            StreamKind::Activity,
            Some(StreamPayload {
                outcome: Some(outcome.clone()),
                summary: Some(event.kind().to_owned()),
                ..StreamPayload::default()
            }),
        ),
        _ => (
            StreamKind::Activity,
            Some(StreamPayload {
                summary: Some(event.kind().to_owned()),
                ..StreamPayload::default()
            }),
        ),
    }
}
