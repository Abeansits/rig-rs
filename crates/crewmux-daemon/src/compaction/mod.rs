//! Claude compaction: hook intake, manual trigger, and restore delivery.
//!
//! Every paste goes through [`crate::submit::submit_text`], which requires
//! `Ready` and waits on [`crate::lineage::ensure`].

mod apply;
mod http;
mod map;
mod persist;
mod spool;
mod trigger;

#[cfg(test)]
mod settle;
#[cfg(test)]
mod skip_map;

use std::collections::BTreeMap;

use crewmux_health::compaction::{Book, Enforcer, PreparationState, Threshold};
use serde_json::Value;

use crate::state::App;

pub(crate) use http::routes;
pub(crate) use persist::{load, save};
pub(crate) use spool::drain_spools;

/// Record a Claude pane so a later compaction paste passes `lineage::ensure`.
///
/// # Errors
///
/// Returns a display string when the seat, pane, or lineage row cannot be stored.
#[cfg(test)]
async fn remember_launched_claude(
    app: &App,
    mux: &crewmux_mux::FakeMux,
    session_id: &str,
) -> Result<(), String> {
    use crewmux_mux::Multiplexer;
    let seat = crate::query::find_seat(app, "claude")
        .await
        .map_err(|err| err.to_string())?;
    let cwd = seat.cwd.clone().unwrap_or_default();
    let name = seat
        .tmux_session
        .clone()
        .unwrap_or_else(|| "seat__rig".to_owned());
    let spec = crewmux_mux::SessionSpec::new(name, cwd);
    let pane = mux
        .create_session(&spec)
        .await
        .map_err(|err| err.to_string())?;
    mux.set_pane_process(
        &pane.session,
        Some(1),
        "claude",
        Some(&format!("claude --session-id {session_id}")),
    )
    .await;
    crate::lineage::record(app, &seat, &pane)
        .await
        .map_err(|err| err.to_string())
}

#[derive(Clone)]
pub(crate) struct CompactPlan {
    instruction: String,
    used_percent: Option<u8>,
    threshold: Threshold,
}

pub(crate) struct Runtime {
    book: Book,
    enforcer: Enforcer,
    preps: BTreeMap<String, PreparationState>,
    plans: BTreeMap<String, CompactPlan>,
}

impl Runtime {
    pub(crate) fn new() -> Self {
        Self {
            book: Book::new(),
            enforcer: Enforcer::new(),
            preps: BTreeMap::new(),
            plans: BTreeMap::new(),
        }
    }

    fn plan(&self, session: &str) -> Option<&CompactPlan> {
        self.plans.get(session)
    }

    fn set_plan(&mut self, session: &str, plan: CompactPlan) {
        self.plans.insert(session.to_owned(), plan);
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
        let released = trigger::release_finished(&mut guard);
        trigger::expire_preps(&mut guard, crate::state::now_ms());
        expired || released
    };
    if changed {
        save(app).await?;
    }
    map::publish(app).await?;
    trigger::deliver_prepared(app).await?;
    apply::deliver_due(app).await
}
