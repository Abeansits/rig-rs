//! Claude compaction: hook intake, manual trigger, and restore delivery.
//!
//! Sends go through [`crate::submit::submit_text`], which requires `Ready`.

mod apply;
mod http;
mod persist;
mod spool;
mod trigger;

use std::collections::BTreeMap;

use crewmux_health::compaction::{Book, Enforcer, PreparationState};
use serde_json::Value;

use crate::state::App;

pub(crate) use http::routes;
pub(crate) use persist::{load, save};
pub(crate) use spool::drain_spools;

pub(crate) struct Runtime {
    book: Book,
    enforcer: Enforcer,
    preps: BTreeMap<String, PreparationState>,
}

impl Runtime {
    pub(crate) fn new() -> Self {
        Self {
            book: Book::new(),
            enforcer: Enforcer::new(),
            preps: BTreeMap::new(),
        }
    }

    fn prep(&self, session: &str) -> PreparationState {
        self.preps
            .get(session)
            .cloned()
            .unwrap_or_else(PreparationState::idle)
    }

    fn set_prep(&mut self, session: &str, prep: PreparationState) {
        self.preps.insert(session.to_owned(), prep);
    }
}

pub(crate) async fn summaries(app: &App) -> Vec<Value> {
    let guard = app.compaction.lock().await;
    guard
        .book
        .attempts()
        .iter()
        .map(crewmux_health::compaction::summary_value)
        .collect()
}

pub(crate) async fn supervise(app: App) {
    loop {
        tokio::select! {
            () = app.cancel.cancelled() => break,
            () = tokio::time::sleep(std::time::Duration::from_secs(2)) => {
                if let Err(err) = tick(&app).await {
                    tracing::warn!(%err, "compaction tick");
                }
            }
        }
    }
}

async fn tick(app: &App) -> Result<(), crate::error::Error> {
    let changed = {
        let mut guard = app.compaction.lock().await;
        let expired = guard.book.tick(crate::state::now_ms());
        trigger::expire_preps(&mut guard, crate::state::now_ms());
        expired
    };
    if changed {
        save(app).await?;
    }
    apply::deliver_due(app).await
}
