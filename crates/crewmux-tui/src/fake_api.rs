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

        let socket = path.clone();
        let got = tokio::task::spawn_blocking(move || Client::over_uds(&socket).hydrate())
            .await
            .expect("join")
            .expect("hydrate");
        assert_eq!(got.seats.len(), 1);
        assert_eq!(got.seats[0].name.as_str(), "alpha");
        assert_eq!(got.seats[0].state, crewmux_api::SeatState::Ready);

        let info =
            tokio::task::spawn_blocking(move || Client::over_uds(&path).attach_argv("alpha"))
                .await
                .expect("join")
                .expect("attach");
        assert_eq!(info, vec!["tmux".to_owned(), "attach".to_owned()]);

        cancel.cancel();
        task.await.expect("server task").expect("serve");
    }
}
