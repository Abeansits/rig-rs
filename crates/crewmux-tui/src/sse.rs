// Derived from openrig packages/tui/src/live-events.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Client for `GET /api/events`. This module decodes [`crewmux_api::StreamEvent`] only.
//! It does not link the daemon. Unknown `type` tags are skipped so slice 4 can extend the stream.

use std::pin::Pin;
use std::time::Duration;

use futures_util::Stream;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crewmux_api::StreamEvent;
use serde_json::Value;

use crate::state::ConnState;

/// Cap retained by the TypeScript parser when a frame never closes.
const MAX_FRAME_CHARS: usize = 1_048_576;
/// Backoff ceiling from the TypeScript client.
const RECONNECT_CAP: Duration = Duration::from_secs(30);

/// What one open attempt returned.
pub(crate) enum Opened {
    /// The server is not offering the event stream. Do not retry.
    Unavailable,
    /// A temporary open failure. Retry with backoff.
    Failed,
    /// A byte stream of SSE frames. `None` from the stream is a clean close.
    Chunks(Pin<Box<dyn Stream<Item = Result<Vec<u8>, ()>> + Send>>),
}

/// Spawn the reconnect loop. `open` is called on each attempt.
pub(crate) fn spawn<F, Fut>(
    tracker: &TaskTracker,
    cancel: CancellationToken,
    base: Duration,
    open: F,
    events: mpsc::Sender<StreamEvent>,
    status: mpsc::Sender<ConnState>,
) where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Opened> + Send + 'static,
{
    let (notes_tx, mut notes_rx) = mpsc::channel(32);
    let forward_cancel = cancel.clone();
    tracker.spawn(async move {
        loop {
            let note = tokio::select! {
                () = forward_cancel.cancelled() => return,
                note = notes_rx.recv() => note,
            };
            let Some(note) = note else { return };
            let Ok(event) = serde_json::from_value::<StreamEvent>(note) else {
                continue;
            };
            if events.send(event).await.is_err() {
                return;
            }
        }
    });
    tracker.spawn(async move {
        connect_loop(cancel, base, open, notes_tx, status).await;
    });
}

pub(crate) async fn connect_loop<F, Fut>(
    cancel: CancellationToken,
    base: Duration,
    open: F,
    events: mpsc::Sender<Value>,
    status: mpsc::Sender<ConnState>,
) where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Opened>,
{
    let mut delay = base;
    loop {
        if cancel.is_cancelled() {
            return;
        }
        match open().await {
            Opened::Unavailable => {
                let _ = status.send(ConnState::Unavailable).await;
                return;
            }
            Opened::Failed => {}
            Opened::Chunks(stream) => {
                let _ = status.send(ConnState::Connected).await;
                let saw_event = read_stream(stream, &cancel, &events).await;
                if saw_event {
                    delay = base;
                }
            }
        }
        if cancel.is_cancelled() {
            return;
        }
        let _ = status.send(ConnState::Dropped).await;
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(delay) => {}
        }
        delay = (delay.saturating_mul(2)).min(RECONNECT_CAP);
        if cancel.is_cancelled() {
            return;
        }
        let _ = status.send(ConnState::Reconnecting).await;
    }
}

async fn read_stream(
    mut stream: Pin<Box<dyn Stream<Item = Result<Vec<u8>, ()>> + Send>>,
    cancel: &CancellationToken,
    events: &mpsc::Sender<Value>,
) -> bool {
    use futures_util::StreamExt;
    let mut parser = Parser::default();
    let mut saw = false;
    loop {
        let chunk = tokio::select! {
            () = cancel.cancelled() => return saw,
            next = stream.next() => next,
        };
        let Some(chunk) = chunk else {
            return saw;
        };
        let Ok(bytes) = chunk else {
            return saw;
        };
        if parser.push(&bytes, events).await {
            saw = true;
        }
    }
}

#[derive(Default)]
struct Parser {
    buf: String,
    data: Vec<String>,
}

impl Parser {
    async fn push(&mut self, chunk: &[u8], events: &mpsc::Sender<Value>) -> bool {
        let text = String::from_utf8_lossy(chunk);
        self.buf.push_str(&text);
        if self.buf.len() > MAX_FRAME_CHARS {
            self.buf.clear();
            self.data.clear();
            return false;
        }
        let mut saw = false;
        while let Some(pos) = self.buf.find('\n') {
            let Some(line) = self.buf.get(..pos) else {
                break;
            };
            let line = line.trim_end_matches('\r').to_owned();
            let rest = self
                .buf
                .get(pos.saturating_add(1)..)
                .unwrap_or("")
                .to_owned();
            self.buf = rest;
            if line.is_empty() {
                if let Some(event) = finish_frame(&self.data) {
                    if events.send(event).await.is_err() {
                        return saw;
                    }
                    saw = true;
                }
                self.data.clear();
            } else if let Some(value) = data_field(&line) {
                self.data.push(value);
            }
        }
        saw
    }
}

fn data_field(line: &str) -> Option<String> {
    if line == "data" {
        return Some(String::new());
    }
    let value = line.strip_prefix("data:")?;
    Some(value.strip_prefix(' ').unwrap_or(value).to_owned())
}

/// Open `GET /api/events` on a Unix socket. The TUI calls this; it does not construct the server.
pub(crate) async fn open_unix(path: &std::path::Path) -> Opened {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let Ok(mut stream) = tokio::net::UnixStream::connect(path).await else {
        return Opened::Failed;
    };
    let request =
        b"GET /api/events HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\n\r\n";
    if stream.write_all(request).await.is_err() {
        return Opened::Failed;
    }
    let mut buf = Vec::new();
    let mut tmp = [0_u8; 1024];
    let header_at = loop {
        match stream.read(&mut tmp).await {
            Ok(0) | Err(_) => return Opened::Failed,
            Ok(n) => buf.extend_from_slice(tmp.get(..n).unwrap_or(&[])),
        }
        if let Some(pos) = header_end(&buf) {
            break pos;
        }
        if buf.len() > 64 * 1024 {
            return Opened::Failed;
        }
    };
    let head = String::from_utf8_lossy(buf.get(..header_at).unwrap_or(&[]));
    if !head.contains("200") || !head.to_ascii_lowercase().contains("text/event-stream") {
        return Opened::Unavailable;
    }
    let rest = buf.get(header_at..).unwrap_or(&[]).to_vec();
    Opened::Chunks(byte_stream(stream, rest))
}

fn header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|pos| pos.saturating_add(4))
}

fn byte_stream(
    stream: tokio::net::UnixStream,
    rest: Vec<u8>,
) -> Pin<Box<dyn Stream<Item = Result<Vec<u8>, ()>> + Send>> {
    use tokio::io::AsyncReadExt;
    Box::pin(futures_util::stream::unfold(
        (stream, Some(rest)),
        |(mut stream, pending)| async move {
            if let Some(rest) = pending.filter(|chunk| !chunk.is_empty()) {
                return Some((Ok(rest), (stream, None)));
            }
            let mut tmp = [0_u8; 1024];
            match stream.read(&mut tmp).await {
                Ok(0) | Err(_) => None,
                Ok(n) => {
                    let chunk = tmp.get(..n).unwrap_or(&[]).to_vec();
                    Some((Ok(chunk), (stream, None)))
                }
            }
        },
    ))
}

fn finish_frame(data: &[String]) -> Option<Value> {
    let raw = data.join("\n");
    if raw.trim().is_empty() {
        return None;
    }
    // Any JSON object resets backoff. The app keeps only [`StreamEvent`] and
    // ignores tags slice 4 may add.
    serde_json::from_str(&raw).ok()
}
