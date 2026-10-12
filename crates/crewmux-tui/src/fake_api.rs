//! In-process stand-in for slice 1's seat list and attach route.

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::routing::{get, post};
use tokio::net::UnixListener;
use tokio_util::sync::CancellationToken;

/// `GET /api/seats` body the fake returns.
#[derive(Clone)]
struct Fixture {
    body: serde_json::Value,
    argv: Vec<String>,
}

fn router(fixture: Fixture) -> Router {
    Router::new()
        .route("/api/seats", get(seats))
        .route("/api/seats/attach", post(attach))
        .with_state(fixture)
}

async fn seats(State(fixture): State<Fixture>) -> Json<serde_json::Value> {
    Json(fixture.body)
}

async fn attach(State(fixture): State<Fixture>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"argv": fixture.argv}))
}

async fn serve(
    listener: UnixListener,
    fixture: Fixture,
    cancel: CancellationToken,
) -> Result<(), std::io::Error> {
    axum::serve(listener, router(fixture))
        .with_graceful_shutdown(async move {
            cancel.cancelled().await;
        })
        .await
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crewmux_api::Client;

    use super::{CancellationToken, Fixture, UnixListener, serve};

    #[tokio::test]
    async fn client_reads_slice1_shapes_from_the_fake_server() {
        let fixture = Fixture {
            body: serde_json::json!({
                "rigs": [{
                    "name": "demo",
                    "seats": [{
                        "name": "alpha",
                        "runtime": "stub",
                        "state": "ready",
                        "tmux_session": "seat"
                    }]
                }]
            }),
            argv: vec!["tmux".to_owned(), "attach".to_owned()],
        };
        let dir = tempfile::tempdir().expect("temp");
        let path = dir.path().join("api.sock");
        let listener = UnixListener::bind(&path).expect("bind");
        let cancel = CancellationToken::new();
        let task = tokio::spawn(serve(listener, fixture, cancel.clone()));
        // The accept task may not have been polled yet. A first connect can
        // see a broken pipe; retry off the worker until the listener is up.
        tokio::task::yield_now().await;

        let socket = path.clone();
        let got = retry(move || Client::over_uds(&socket).hydrate()).await;
        assert_eq!(got.seats.len(), 1);
        assert_eq!(got.seats[0].name.as_str(), "alpha");
        assert_eq!(got.seats[0].state, crewmux_api::SeatState::Ready);

        let info = retry(move || Client::over_uds(&path).attach_argv("alpha")).await;
        assert_eq!(info, vec!["tmux".to_owned(), "attach".to_owned()]);

        cancel.cancel();
        task.await.expect("server task").expect("serve");
    }

    async fn retry<T>(op: impl Fn() -> Result<T, crewmux_api::Error> + Send + Sync + 'static) -> T
    where
        T: Send + 'static,
    {
        let op = std::sync::Arc::new(op);
        let mut last = String::new();
        for _ in 0..8 {
            let op = std::sync::Arc::clone(&op);
            match tokio::task::spawn_blocking(move || op()).await {
                Ok(Ok(value)) => return value,
                Ok(Err(err)) => last = err.to_string(),
                Err(err) => last = err.to_string(),
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("exchange: {last}");
    }
}
