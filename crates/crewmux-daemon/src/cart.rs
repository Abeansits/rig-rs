//! Crash-cart conductor. Kernel rigs first. Cancel stops before the next rig.

use std::collections::HashMap;

use crewmux_core::RigRepo;
use crewmux_durability::{CartWalk, RigRestoreOutcome, kernel_first};
use crewmux_types::{LaunchPosture, RigId};
use serde_json::{Value, json};

use crate::db::read;
use crate::error::Error;
use crate::restore::restore_rig;
use crate::state::{App, join_err};

/// Restore every rig. `cancel` lets the in-flight rig finish and skips the rest.
///
/// The flag is an atomic the in-flight restore can raise. Later rigs are
/// `not_attempted`. One failed rig does not stop the fleet.
///
/// # Errors
///
/// Returns [`Error`] when the rig list cannot be read.
pub(crate) async fn crash_cart(app: &App, cancel: bool) -> Result<Value, Error> {
    if cancel {
        app.request_cart_cancel();
    } else {
        app.clear_cart_cancel();
    }
    let listed = read(&app.store, |conn| conn.list()).await?;
    let kernels = kernel_ids(app).await?;
    let pairs: Vec<(RigId, bool)> = listed
        .iter()
        .map(|rig| (rig.id, kernels.iter().any(|id| id == &rig.id.to_string())))
        .collect();
    let ordered = kernel_first(&pairs);
    let names: HashMap<RigId, String> = listed
        .into_iter()
        .map(|rig| (rig.id, rig.name.as_str().to_owned()))
        .collect();
    let mut walk = CartWalk::new();
    while let Some(step) = walk.next(&ordered) {
        let outcome = if step.run {
            one_rig(app, names.get(&step.rig).map_or("", String::as_str)).await
        } else {
            RigRestoreOutcome::NotAttempted
        };
        let stop = !step.run || app.cart_cancel_requested();
        walk.record(&ordered, outcome, stop);
    }
    let run = walk.into_run();
    let rows: Vec<Value> = run
        .rigs
        .iter()
        .map(|rig| {
            json!({
                "rig": names.get(&rig.rig).map_or("", String::as_str),
                "outcome": rig.outcome.as_str(),
                "kernel": rig.kernel,
            })
        })
        .collect();
    let cancelled_before = run
        .cancelled_before
        .as_ref()
        .and_then(|id| names.get(id))
        .map(String::as_str);
    Ok(json!({"rigs": rows, "cancelled_before": cancelled_before}))
}

async fn one_rig(app: &App, name: &str) -> RigRestoreOutcome {
    match restore_rig(app, name, None, LaunchPosture::Auto).await {
        Ok(value) => parse_outcome(
            value
                .get("outcome")
                .and_then(Value::as_str)
                .unwrap_or("failed"),
        ),
        Err(_) => RigRestoreOutcome::Failed,
    }
}

fn parse_outcome(text: &str) -> RigRestoreOutcome {
    match text {
        "fully_restored" => RigRestoreOutcome::FullyRestored,
        "partially_restored" => RigRestoreOutcome::PartiallyRestored,
        "not_attempted" => RigRestoreOutcome::NotAttempted,
        _ => RigRestoreOutcome::Failed,
    }
}

async fn kernel_ids(app: &App) -> Result<Vec<String>, Error> {
    let store = std::sync::Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || store.read(crewmux_store::rigs_kernel))
        .await
        .map_err(join_err)?
        .map_err(Error::from)
}
