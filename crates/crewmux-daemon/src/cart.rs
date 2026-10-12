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

/// Restore every rig. `cancel` does not start a walk.
///
/// When a walk is already in progress, `cancel` sets the flag and returns.
/// The in-flight rig finishes. Later rigs are `not_attempted`. When nothing
/// is running, `cancel` is a no-op with a message. A second walk is refused.
///
/// # Errors
///
/// Returns [`Error`] when a walk is already running, or the rig list cannot be read.
pub(crate) async fn crash_cart(app: &App, cancel: bool) -> Result<Value, Error> {
    if cancel {
        return Ok(cancel_only(app));
    }
    if !app.try_begin_cart() {
        return Err(Error::Message("a crash-cart is already running".to_owned()));
    }
    let _guard = CartGuard { app };
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

fn cancel_only(app: &App) -> Value {
    if app.request_cart_cancel() {
        json!({
            "status": "cancel_requested",
            "message": "the in-flight rig will finish; later rigs will not be restored",
        })
    } else {
        json!({
            "status": "idle",
            "message": "no crash-cart is running",
        })
    }
}

/// Clears the running bit when the walk returns, including on error.
struct CartGuard<'a> {
    app: &'a App,
}

impl Drop for CartGuard<'_> {
    fn drop(&mut self) {
        self.app.end_cart();
    }
}

async fn one_rig(app: &App, name: &str) -> RigRestoreOutcome {
    hold_for_test(app, name).await;
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

/// `$RIG_HOME/cart-hold` names the rig to pause on, under `RIG_TEST_HOOKS=1`.
///
/// The walk writes `$RIG_HOME/cart-at` and waits until the hold file no longer
/// names this rig, then restores it. A cancel during the wait still lets the
/// rig finish.
async fn hold_for_test(app: &App, rig_name: &str) {
    if std::env::var("RIG_TEST_HOOKS").ok().as_deref() != Some("1") {
        return;
    }
    let hold = app.home.join("cart-hold");
    let marker = app.home.join("cart-at");
    for _ in 0..1500 {
        if app.cancel.is_cancelled() {
            return;
        }
        let wanted = read_trim(&hold).await;
        if wanted.as_deref() != Some(rig_name) {
            return;
        }
        let path = marker.clone();
        let name = rig_name.to_owned();
        drop(tokio::task::spawn_blocking(move || std::fs::write(path, name)).await);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

async fn read_trim(path: &std::path::Path) -> Option<String> {
    let path = path.to_path_buf();
    let text = tokio::task::spawn_blocking(move || std::fs::read_to_string(path).ok()).await;
    text.ok()
        .flatten()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

async fn kernel_ids(app: &App) -> Result<Vec<String>, Error> {
    let store = std::sync::Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || store.read(crewmux_store::rigs_kernel))
        .await
        .map_err(join_err)?
        .map_err(Error::from)
}
