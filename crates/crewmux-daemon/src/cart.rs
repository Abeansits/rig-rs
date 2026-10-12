//! Crash-cart conductor. Kernel rigs first. Cancel stops before the next rig.

use std::collections::HashMap;

use crewmux_core::RigRepo;
use crewmux_durability::kernel_first;
use crewmux_types::{LaunchPosture, RigId};
use serde_json::{Value, json};

use crate::db::read;
use crate::error::Error;
use crate::restore::restore_rig;
use crate::state::{App, join_err};

/// Restore every rig. `cancel` lets the first rig finish and skips the rest.
///
/// # Errors
///
/// Returns [`Error`] when the rig list cannot be read. A rig that fails to
/// restore is recorded as `failed` and the fleet continues.
pub(crate) async fn crash_cart(app: &App, cancel: bool) -> Result<Value, Error> {
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
    let mut rows = Vec::new();
    let mut cancelled_before = None;
    let mut stop = false;
    for (id, kernel) in ordered {
        let name = names.get(&id).map_or("", String::as_str);
        if stop {
            if cancelled_before.is_none() {
                cancelled_before = Some(name.to_owned());
            }
            rows.push(json!({"rig": name, "outcome": "not_attempted", "kernel": kernel}));
            continue;
        }
        rows.push(json!({
            "rig": name,
            "outcome": one_rig(app, name).await,
            "kernel": kernel,
        }));
        if cancel {
            stop = true;
        }
    }
    Ok(json!({"rigs": rows, "cancelled_before": cancelled_before}))
}

async fn one_rig(app: &App, name: &str) -> String {
    match restore_rig(app, name, None, LaunchPosture::Auto).await {
        Ok(value) => value
            .get("outcome")
            .and_then(Value::as_str)
            .unwrap_or("failed")
            .to_owned(),
        Err(_) => "failed".to_owned(),
    }
}

async fn kernel_ids(app: &App) -> Result<Vec<String>, Error> {
    let store = std::sync::Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || store.read(crewmux_store::rigs_kernel))
        .await
        .map_err(join_err)?
        .map_err(Error::from)
}
