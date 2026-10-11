// Derived from openrig packages/tui/src/live-events.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Client for `GET /api/events`. This module decodes [`crewmux_api::StreamEvent`] only.
//! It does not link the daemon. Unknown `type` values are skipped. A reconnect sends
//! the last id as `Last-Event-ID` and `last_event_id`. A `gap` event is forwarded
//! so the app can re-hydrate.

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
    F: Fn(Option<u64>) -> Fut + Send + Sync + 'static,
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
    F: Fn(Option<u64>) -> Fut,
    Fut: std::future::Future<Output = Opened>,
{
    let mut delay = base;
    let mut last_id = None;
    loop {
        if cancel.is_cancelled() {
            return;
        }
        let opened = tokio::select! {
            () = cancel.cancelled() => return,
            opened = open(last_id) => opened,
        };
        match opened {
            Opened::Unavailable => {
                let _ = status.send(ConnState::Unavailable).await;
                return;
            }
            Opened::Failed => {}
            Opened::Chunks(stream) => {
                let _ = status.send(ConnState::Connected).await;
                let (saw_event, seen) = read_stream(stream, &cancel, &events).await;
                if let Some(id) = seen {
                    last_id = Some(id);
                }
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
) -> (bool, Option<u64>) {
    use futures_util::StreamExt;
    let mut parser = Parser::default();
    let mut saw = false;
    loop {
        let chunk = tokio::select! {
            () = cancel.cancelled() => return (saw, parser.last_id),
            next = stream.next() => next,
        };
        let Some(chunk) = chunk else {
            return (saw, parser.last_id);
        };
        let Ok(bytes) = chunk else {
            return (saw, parser.last_id);
        };
        if parser.push(&bytes, events).await {
            saw = true;
        }
    }
}

#[derive(Default)]
struct Parser {
    /// Bytes that are not yet a complete UTF-8 sequence.
    pending: Vec<u8>,
    buf: String,
    data: Vec<String>,
    /// SSE `id:` for the frame currently being built.
    id: Option<u64>,
    /// Id of the last finished frame that carried one.
    last_id: Option<u64>,
    /// The current frame exceeded [`MAX_FRAME_CHARS`] and is discarded until the blank line.
    skip_frame: bool,
}

impl Parser {
    async fn push(&mut self, chunk: &[u8], events: &mpsc::Sender<Value>) -> bool {
        self.pending.extend_from_slice(chunk);
        self.buf.push_str(&take_utf8(&mut self.pending));
        if !self.buf.contains('\n') && self.buf.len() > MAX_FRAME_CHARS {
            self.buf.clear();
            self.data.clear();
            self.id = None;
            self.skip_frame = true;
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
            if frame_over_cap(&self.data, &line) {
                self.data.clear();
                self.id = None;
                self.skip_frame = true;
            }
            if line.is_empty() {
                if !self.skip_frame
                    && let Some((event, id)) = finish_frame(&self.data, self.id.take())
                {
                    if let Some(id) = id {
                        self.last_id = Some(id);
                    }
                    if events.send(event).await.is_err() {
                        return saw;
                    }
                    saw = true;
                }
                self.id = None;
                self.data.clear();
                self.skip_frame = false;
            } else if !self.skip_frame {
                if let Some(id) = id_field(&line) {
                    self.id = Some(id);
                } else if let Some(value) = data_field(&line) {
                    self.data.push(value);
                }
            }
        }
        saw
    }
}

fn frame_over_cap(data: &[String], line: &str) -> bool {
    let mut total = line.len();
    for part in data {
        total = total.saturating_add(part.len());
    }
    total > MAX_FRAME_CHARS
}

/// Decode complete UTF-8 and keep a trailing incomplete sequence for the next chunk.
fn take_utf8(pending: &mut Vec<u8>) -> String {
    match std::str::from_utf8(pending) {
        Ok(text) => {
            let owned = text.to_owned();
            pending.clear();
            owned
        }
        Err(err) => {
            let valid = err.valid_up_to();
            let head = pending.get(..valid).unwrap_or(&[]);
            let mut text = match std::str::from_utf8(head) {
                Ok(text) => text.to_owned(),
                Err(_) => String::new(),
            };
            match err.error_len() {
                None => {
                    pending.drain(..valid);
                }
                Some(len) => {
                    let end = valid.saturating_add(len).min(pending.len());
                    pending.drain(..end);
                    text.push('\u{FFFD}');
                }
            }
            text
        }
    }
}

enum HeadClass {
    Stream,
    Unavailable,
    Retry,
}

fn head_class(head: &str) -> HeadClass {
    let sse = head.to_ascii_lowercase().contains("text/event-stream");
    match status_code(head) {
        Some(code) if (200..300).contains(&code) && sse => HeadClass::Stream,
        Some(code) if (200..300).contains(&code) => HeadClass::Unavailable,
        Some(_) | None => HeadClass::Retry,
    }
}

fn status_code(head: &str) -> Option<u16> {
    let line = head.lines().next()?;
    let mut parts = line.split_whitespace();
    let _http = parts.next()?;
    parts.next()?.parse().ok()
}

fn data_field(line: &str) -> Option<String> {
    if line == "data" {
        return Some(String::new());
    }
    let value = line.strip_prefix("data:")?;
    Some(value.strip_prefix(' ').unwrap_or(value).to_owned())
}

fn event_request(last_id: Option<u64>) -> String {
    match last_id {
        None => "GET /api/events HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\n\r\n"
            .to_owned(),
        Some(id) => format!(
            "GET /api/events?last_event_id={id} HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\nLast-Event-ID: {id}\r\n\r\n"
        ),
    }
}

/// Open `GET /api/events` on a Unix socket. The TUI calls this; it does not construct the server.
///
/// `last_id` is sent on reconnect as both the SSE header and the query name slice 4 uses.
pub(crate) async fn open_unix(path: &std::path::Path, last_id: Option<u64>) -> Opened {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let Ok(mut stream) = tokio::net::UnixStream::connect(path).await else {
        return Opened::Failed;
    };
    let request = event_request(last_id);
    if stream.write_all(request.as_bytes()).await.is_err() {
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
    match head_class(&head) {
        HeadClass::Stream => {}
        HeadClass::Unavailable => return Opened::Unavailable,
        HeadClass::Retry => return Opened::Failed,
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

fn id_field(line: &str) -> Option<u64> {
    let value = line.strip_prefix("id:")?;
    value.trim().parse().ok()
}

fn finish_frame(data: &[String], sse_id: Option<u64>) -> Option<(Value, Option<u64>)> {
    let raw = data.join("\n");
    if raw.trim().is_empty() {
        return None;
    }
    // Any JSON object resets backoff. The app keeps only [`StreamEvent`] and
    // ignores tags slice 4 may add.
    let value: Value = serde_json::from_str(&raw).ok()?;
    let id = sse_id.or_else(|| value.get("id").and_then(Value::as_u64));
    Some((value, id))
}

#[cfg(test)]
mod tests {
    use super::{HeadClass, Parser, event_request, head_class, take_utf8};

    #[test]
    fn resume_request_names_last_event_id() {
        let first = event_request(None);
        assert!(first.starts_with("GET /api/events HTTP/1.1\r\n"));
        assert!(!first.contains("Last-Event-ID"));
        assert!(!first.contains("last_event_id"));
        let again = event_request(Some(4));
        assert!(again.contains("GET /api/events?last_event_id=4 "));
        assert!(again.contains("Last-Event-ID: 4\r\n"));
    }

    #[test]
    fn non_200_retries_and_a_json_200_does_not() {
        assert!(matches!(
            head_class("HTTP/1.1 503 Service Unavailable\r\n\r\n"),
            HeadClass::Retry
        ));
        assert!(matches!(
            head_class("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n"),
            HeadClass::Unavailable
        ));
        assert!(matches!(
            head_class("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n"),
            HeadClass::Stream
        ));
    }

    #[test]
    fn utf8_sequences_span_chunks() {
        let mut pending = vec![0xC3];
        assert_eq!(take_utf8(&mut pending), "");
        assert_eq!(pending, vec![0xC3]);
        pending.push(0xA9);
        assert_eq!(take_utf8(&mut pending), "é");
        assert_eq!(pending, Vec::<u8>::new());
    }

    #[tokio::test]
    async fn oversized_frame_is_dropped_and_the_next_frame_parses() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(2);
        let mut parser = Parser::default();
        let mut huge = b"data: ".to_vec();
        huge.extend(std::iter::repeat_n(b'x', super::MAX_FRAME_CHARS));
        huge.extend(b"\n\n");
        assert!(!parser.push(&huge, &tx).await);
        let frame = b"data: {\"id\":2,\"type\":\"gap\"}\n\n";
        assert!(parser.push(frame, &tx).await);
        let value = rx.try_recv().expect("frame");
        assert_eq!(value.get("id").and_then(serde_json::Value::as_u64), Some(2));
    }
}
