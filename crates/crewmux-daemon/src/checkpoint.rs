//! Agent checkpoints. Writes go through the store writer.

use std::sync::Arc;

use crewmux_store::CheckpointRow;
use crewmux_types::SeatId;
use serde_json::{Value, json};

use crate::error::Error;
use crate::state::{App, join_err, now_ms};

/// Fields a checkpoint write carries.
pub(crate) struct CheckpointDraft {
    /// Seat name.
    pub seat: String,
    /// Summary.
    pub summary: String,
    /// Current task.
    pub current_task: Option<String>,
    /// Next step.
    pub next_step: Option<String>,
    /// Blocked on.
    pub blocked_on: Option<String>,
    /// Artifact paths.
    pub key_artifacts: Vec<String>,
    /// Confidence label.
    pub confidence: Option<String>,
}

/// Persist a checkpoint for a seat name.
///
/// # Errors
///
/// Returns [`Error`] when the seat is missing or the row cannot be written.
pub(crate) async fn write_checkpoint(app: &App, draft: CheckpointDraft) -> Result<Value, Error> {
    let seat = crate::query::find_seat(app, &draft.seat).await?;
    let artifacts = serde_json::to_string(&draft.key_artifacts)
        .map_err(|err| Error::Message(err.to_string()))?;
    let row = CheckpointRow {
        id: format!("checkpoint-{}", SeatId::new()),
        seat_id: seat.id.to_string(),
        summary: draft.summary,
        current_task: draft.current_task,
        next_step: draft.next_step,
        blocked_on: draft.blocked_on,
        key_artifacts: artifacts,
        confidence: draft.confidence,
        created_ms: i64::try_from(now_ms()).unwrap_or(0),
    };
    let rendered = checkpoint_json(&row, &draft.seat);
    let store = Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || {
        store.write(move |conn| crewmux_store::insert_checkpoint(conn, &row))
    })
    .await
    .map_err(join_err)?
    .map_err(Error::from)?;
    Ok(rendered)
}

/// Latest checkpoint for a seat, or null.
///
/// # Errors
///
/// Returns [`Error`] when the seat is missing.
pub(crate) async fn show_checkpoint(app: &App, seat_name: &str) -> Result<Value, Error> {
    let seat = crate::query::find_seat(app, seat_name).await?;
    let id = seat.id.to_string();
    let store = Arc::clone(&app.store);
    let rows = tokio::task::spawn_blocking(move || {
        store.read(move |conn| crewmux_store::list_checkpoints(conn, &id))
    })
    .await
    .map_err(join_err)??;
    let Some(row) = rows.last() else {
        return Ok(json!({"seat": seat_name, "checkpoint": null}));
    };
    Ok(checkpoint_json(row, seat_name))
}

fn checkpoint_json(row: &CheckpointRow, seat_name: &str) -> Value {
    let artifacts: Vec<String> = serde_json::from_str(&row.key_artifacts).unwrap_or_default();
    json!({
        "id": row.id,
        "seat": seat_name,
        "summary": row.summary,
        "current_task": row.current_task,
        "next_step": row.next_step,
        "blocked_on": row.blocked_on,
        "key_artifacts": artifacts,
        "confidence": row.confidence,
    })
}
