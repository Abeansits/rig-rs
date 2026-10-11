//! Shared Unix-socket harness for the chat route tests.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crewmux_adapters::{AdapterRegistry, StubAdapter};
use crewmux_api::Client;
use crewmux_core::chat::{ChatRepo, NewChat};
use crewmux_core::{Rig, RigRepo, Seat, SeatRepo, SeatState, Timestamp};
use crewmux_mux::{FakeMux, Multiplexer};
use crewmux_store::put_seat_token;
use crewmux_types::{
    RigId, RigName, RuntimeKind, SEAT_ID_HEADER, SEAT_TOKEN_HEADER, SeatId, SeatName, SenderRef,
};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

pub(super) struct Harness {
    pub(super) app: crate::state::App,
    pub(super) mux: Arc<FakeMux>,
    pub(super) sock: PathBuf,
    dir: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.app.cancel.cancel();
        drop(std::fs::remove_file(&self.sock));
        drop(std::fs::remove_dir_all(&self.dir));
    }
}

pub(super) async fn boot() -> Harness {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "rig-chat-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("dir");
    let sock = dir.join("d.sock");
    let mux = Arc::new(FakeMux::with_socket("rig-rs"));
    mux.enable_stub_scripts().await;
    let store = Arc::new(crewmux_store::Store::open(std::path::Path::new(":memory:")).expect("db"));
    let cloned = Arc::clone(&mux);
    let shared: Arc<dyn Multiplexer> = cloned;
    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(StubAdapter::new(
        Arc::clone(&shared),
        "rig-stub-agent",
    )));
    let app = crate::state::App::new(
        store,
        shared,
        None,
        CancellationToken::new(),
        Arc::new(registry),
    );
    let listener = crate::runtime::bind_socket(&sock).expect("bind");
    let served = app.clone();
    let cancel = app.cancel.clone();
    tokio::spawn(async move {
        drop(
            axum::serve(listener, crate::http::router(served))
                .with_graceful_shutdown(async move { cancel.cancelled().await })
                .await,
        );
    });
    Harness {
        app,
        mux,
        sock,
        dir,
    }
}

pub(super) async fn rig(app: &crate::state::App, name: &str) -> RigId {
    let id = RigId::new();
    let saved = id;
    let label = name.to_owned();
    let store = Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || {
        store.write(move |conn| {
            RigRepo::upsert(
                &crewmux_store::Db::new(conn),
                &Rig {
                    id: saved,
                    name: RigName::new(label).expect("name"),
                    created_at: Timestamp::from_millis(0),
                },
            )?;
            Ok(())
        })
    })
    .await
    .expect("join")
    .expect("rig");
    id
}

pub(super) async fn seed(app: &crate::state::App, rig: RigId, who: &str, body: &str) -> String {
    let who = who.to_owned();
    let body = body.to_owned();
    let store = Arc::clone(&app.store);
    let message = tokio::task::spawn_blocking(move || {
        store.post(NewChat {
            rig,
            sender: SenderRef::parse(&who).expect("sender"),
            body,
            at: Timestamp::from_millis(1_700_000_000_000),
        })
    })
    .await
    .expect("join")
    .expect("post");
    message.id.to_string()
}

pub(super) async fn stamp(app: &crate::state::App, id: &str, at: &str) {
    let id = id.to_owned();
    let at = crewmux_core::chat::parse_since(at, Timestamp::from_millis(0))
        .expect("stamp")
        .millis();
    let store = Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || {
        store.write(move |conn| {
            conn.execute(
                "UPDATE chat_messages SET created_ms = ?1 WHERE id = ?2",
                rusqlite::params![at, id],
            )?;
            Ok(())
        })
    })
    .await
    .expect("join")
    .expect("stamp");
}

pub(super) async fn exchange(
    sock: &std::path::Path,
    method: &str,
    path: &str,
    body: Option<Vec<u8>>,
    headers: Vec<(String, String)>,
) -> crewmux_api::Response {
    let sock = sock.to_path_buf();
    let method = method.to_owned();
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || {
        let client = Client::over_uds(sock);
        let view: Vec<(&str, &str)> = headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        let mut last = String::new();
        for _ in 0..8 {
            match client.request_with_headers(&method, &path, body.as_deref(), &view) {
                Ok(response) => return response,
                Err(err) if method == "GET" || refused(&err) => last = err.to_string(),
                Err(err) => panic!("http: {err}"),
            }
        }
        panic!("http: {last}");
    })
    .await
    .expect("join")
}

fn refused(err: &crewmux_api::Error) -> bool {
    let (crewmux_api::Error::Connect { source, .. } | crewmux_api::Error::Exchange { source, .. }) =
        err
    else {
        return false;
    };
    source.kind() == std::io::ErrorKind::ConnectionRefused
}

pub(super) fn json(body: &str) -> Value {
    serde_json::from_str(body).expect("json")
}

pub(super) fn enc(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = std::fmt::Write::write_fmt(&mut out, format_args!("%{byte:02X}"));
        }
    }
    out
}

pub(super) fn history_path(rig: RigId, query: &str) -> String {
    if query.is_empty() {
        format!("/api/rigs/{rig}/chat/history")
    } else {
        format!("/api/rigs/{rig}/chat/history?{query}")
    }
}

pub(super) async fn seated(app: &crate::state::App, rig: RigId) -> (SeatId, String) {
    let seat = SeatId::new();
    let token = "seat-token".to_owned();
    let saved_seat = seat;
    let saved_token = token.clone();
    let store = Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || {
        store.write(move |conn| {
            let db = crewmux_store::Db::new(conn);
            SeatRepo::upsert(
                &db,
                &Seat {
                    id: saved_seat,
                    rig_id: rig,
                    name: SeatName::new("one").expect("seat"),
                    runtime: RuntimeKind::Stub,
                    cwd: None,
                    model: None,
                    state: SeatState::Ready,
                    reason: None,
                    tmux_session: None,
                    pane_id: None,
                },
            )?;
            put_seat_token(&db, &saved_seat, &saved_token)?;
            Ok(())
        })
    })
    .await
    .expect("join")
    .expect("seat");
    (seat, token)
}

pub(super) fn seat_headers(seat: &SeatId, token: &str) -> Vec<(String, String)> {
    vec![
        (SEAT_ID_HEADER.to_owned(), seat.to_string()),
        (SEAT_TOKEN_HEADER.to_owned(), token.to_owned()),
    ]
}
