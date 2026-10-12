//! In-process stand-in for slice 1's seat list and attach route.

use axum::Json;
use axum::Router;
use axum::body::Bytes;
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

async fn attach(State(fixture): State<Fixture>, body: Bytes) -> Json<serde_json::Value> {
    // Read the POST body before responding. The daemon attach route does this.
    // Dropping the body unread lets the server close the socket while the
    // client is still writing it, which is a broken pipe on that write.
    drop(body);
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
    use std::io::ErrorKind;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use crewmux_api::{Client, Error, Snapshot};

    use super::{CancellationToken, Fixture, UnixListener, serve};

    fn hydrate_at(path: &Path) -> Result<Snapshot, Error> {
        Client::over_uds(path).hydrate()
    }

    fn attach_at(path: &Path) -> Result<Vec<String>, Error> {
        Client::over_uds(path).attach_argv("alpha")
    }

    /// The fake server shares the test runtime. Under a loaded CI job a reset
    /// can land on the first exchange. Retry those, and wait on the runtime
    /// between tries so the server task can accept.
    async fn until_ready<T>(path: PathBuf, op: fn(&Path) -> Result<T, Error>) -> Result<T, Error>
    where
        T: Send + 'static,
    {
        const TRIES: u8 = 8;
        let mut attempt = 0_u8;
        loop {
            let socket = path.clone();
            let joined = tokio::task::spawn_blocking(move || op(&socket))
                .await
                .map_err(|_| Error::Exchange {
                    target: path.display().to_string(),
                    source: std::io::Error::other("blocking task failed"),
                })?;
            match joined {
                Ok(value) => return Ok(value),
                Err(err) if io_blip(&err) && attempt < TRIES.saturating_sub(1) => {
                    attempt = attempt.saturating_add(1);
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(err) => return Err(err),
            }
        }
    }

    fn io_blip(err: &Error) -> bool {
        let (Error::Connect { source, .. } | Error::Exchange { source, .. }) = err else {
            return false;
        };
        matches!(
            source.kind(),
            ErrorKind::BrokenPipe
                | ErrorKind::ConnectionReset
                | ErrorKind::ConnectionAborted
                | ErrorKind::ConnectionRefused
                | ErrorKind::NotConnected
                | ErrorKind::TimedOut
                | ErrorKind::Interrupted
        )
    }

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

        let got = until_ready(path.clone(), hydrate_at)
            .await
            .expect("hydrate");
        assert_eq!(got.seats.len(), 1);
        assert_eq!(got.seats[0].name.as_str(), "alpha");
        assert_eq!(got.seats[0].state, crewmux_api::SeatState::Ready);

        let info = until_ready(path, attach_at).await.expect("attach");
        assert_eq!(info, vec!["tmux".to_owned(), "attach".to_owned()]);

        cancel.cancel();
        task.await.expect("server task").expect("serve");
    }
}
