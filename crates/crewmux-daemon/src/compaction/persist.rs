//! Load and store attempt documents. The enforcer's sample state stays in memory.

use std::sync::Arc;

use crewmux_health::compaction::Attempt;
use crewmux_store::AttemptRow;

use crate::error::Error;
use crate::state::{App, join_err, now_ms};

pub(crate) async fn load(app: &App) -> Result<(), Error> {
    let store = Arc::clone(&app.store);
    let rows = tokio::task::spawn_blocking(move || store.read(crewmux_store::list_attempts))
        .await
        .map_err(join_err)??;
    let mut attempts = Vec::new();
    for row in rows {
        match serde_json::from_str::<Attempt>(&row.doc) {
            Ok(attempt) => attempts.push(attempt),
            Err(err) => tracing::warn!(id = %row.id, %err, "skipped compaction row"),
        }
    }
    app.compaction.lock().await.book.load(attempts);
    Ok(())
}

pub(crate) async fn save(app: &App) -> Result<(), Error> {
    let attempts = app.compaction.lock().await.book.attempts().to_vec();
    let store = Arc::clone(&app.store);
    let updated = i64::try_from(now_ms()).unwrap_or(0);
    tokio::task::spawn_blocking(move || write_rows(&store, attempts, updated))
        .await
        .map_err(join_err)?
}

fn write_rows(
    store: &crewmux_store::Store,
    attempts: Vec<Attempt>,
    updated: i64,
) -> Result<(), Error> {
    store.write(move |conn| {
        for attempt in &attempts {
            let doc = serde_json::to_string(attempt)
                .map_err(|err| crewmux_store::Error::Event(err.to_string()))?;
            crewmux_store::upsert_attempt(
                conn,
                &AttemptRow {
                    id: attempt.id.as_str().to_owned(),
                    seat_key: attempt.seat_key.clone(),
                    updated_ms: updated,
                    doc,
                },
            )?;
        }
        Ok(())
    })?;
    Ok(())
}
