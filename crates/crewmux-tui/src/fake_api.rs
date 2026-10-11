//! In-process stand-in for slice 1's read routes.
//!
//! Slice 1 will mount these paths on the daemon socket. Until then, tests bind
//! this router on a private Unix socket and read it with [`crewmux_api::Client`].
//! It does not serve `GET /api/events`; that temporary fan-out lives in the daemon.

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use tokio::net::UnixListener;
use tokio_util::sync::CancellationToken;

use crewmux_api::{AttachInfo, AttachInfoRoute, Endpoint, Hydrate, ListSeats, SeatList, Snapshot};

/// Fixture the fake server returns for every successful read.
#[derive(Clone)]
struct Fixture {
    snapshot: Snapshot,
    attach: AttachInfo,
}

fn router(fixture: Fixture) -> Router {
    Router::new()
        .route(Hydrate::PATH, get(snapshot))
        .route(ListSeats::PATH, get(seats))
        .route(AttachInfoRoute::PATH, get(attach))
        .with_state(fixture)
}

async fn snapshot(State(fixture): State<Fixture>) -> Json<Snapshot> {
    Json(fixture.snapshot)
}

async fn seats(State(fixture): State<Fixture>) -> Json<SeatList> {
    Json(SeatList {
        seats: fixture.snapshot.seats,
    })
}

async fn attach(
    State(fixture): State<Fixture>,
    Path(seat): Path<String>,
) -> Result<Json<AttachInfo>, StatusCode> {
    if seat == fixture.attach.seat.as_str() {
        Ok(Json(fixture.attach))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
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
    use crewmux_api::{
        Client, PodName, RigId, RigName, RuntimeKind, SeatId, SeatName, SeatState, SeatView,
        SessionId,
    };

    use super::{CancellationToken, Fixture, UnixListener, serve};

    fn sample() -> Fixture {
        let seat = SeatId::parse("s1").expect("seat");
        let session = SessionId::parse("sess").expect("session");
        let view = SeatView {
            id: seat.clone(),
            name: SeatName::parse("alpha").expect("name"),
            rig_id: RigId::parse("rig-1").expect("rig"),
            rig_name: RigName::parse("demo").expect("rig name"),
            pod: PodName::parse("pod").expect("pod"),
            state: SeatState::Ready,
            runtime: RuntimeKind::Stub,
            session: Some(session.clone()),
            spec: Some("demo".to_owned()),
        };
        Fixture {
            snapshot: crewmux_api::Snapshot {
                seats: vec![view],
                health: None,
                activity: Vec::new(),
            },
            attach: crewmux_api::AttachInfo {
                seat,
                socket: "rig-rs".to_owned(),
                session,
            },
        }
    }

    #[tokio::test]
    async fn client_reads_slice1_shapes_from_the_fake_server() {
        let fixture = sample();
        let dir = tempfile::tempdir().expect("temp");
        let path = dir.path().join("api.sock");
        let listener = UnixListener::bind(&path).expect("bind");
        let cancel = CancellationToken::new();
        let task = tokio::spawn(serve(listener, fixture.clone(), cancel.clone()));

        let socket = path.clone();
        let expected = fixture.snapshot.clone();
        let got = tokio::task::spawn_blocking(move || Client::over_uds(&socket).hydrate())
            .await
            .expect("join")
            .expect("hydrate");
        assert_eq!(got, expected);

        let socket = path.clone();
        let listed = tokio::task::spawn_blocking(move || Client::over_uds(&socket).list_seats())
            .await
            .expect("join")
            .expect("seats");
        assert_eq!(listed.seats, expected.seats);

        let info = tokio::task::spawn_blocking(move || Client::over_uds(&path).attach_info("s1"))
            .await
            .expect("join")
            .expect("attach");
        assert_eq!(info, fixture.attach);

        cancel.cancel();
        task.await.expect("server task").expect("serve");
    }
}
