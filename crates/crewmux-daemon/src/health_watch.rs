// Derived from openrig packages/daemon/src/domain/classifier-lease-manager.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Lease TTL watchdog. It marks expired leases. It does not reclaim them.

use std::sync::Arc;
use std::time::Duration;

use crate::health_query::now_i64;
use crate::state::App;

/// Run until the daemon's cancellation token fires.
pub(crate) async fn run(app: App) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    loop {
        tokio::select! {
            () = app.cancel.cancelled() => break,
            _ = interval.tick() => evaluate(&app).await,
        }
    }
}

async fn evaluate(app: &App) {
    let store = Arc::clone(&app.store);
    let now = now_i64();
    let evaluated = tokio::task::spawn_blocking(move || {
        store.write(move |conn| crewmux_store::lease_evaluate(conn, now))
    })
    .await;
    log_eval(evaluated);
}

fn log_eval(evaluated: Result<Result<u32, crewmux_store::Error>, tokio::task::JoinError>) {
    let message = match evaluated {
        Ok(Ok(count)) => {
            tracing::debug!(expired = count, "classifier lease evaluation");
            return;
        }
        Ok(Err(err)) => err.to_string(),
        Err(err) => err.to_string(),
    };
    tracing::warn!(%message, "classifier lease evaluation");
}
