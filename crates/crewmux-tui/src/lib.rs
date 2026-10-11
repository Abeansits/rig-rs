//! Terminal UI backed by the daemon API client.
//!
//! The event reader speaks the canonical `/api/events` envelope.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use crewmux_api::StreamEvent;

/// Read up to `limit` SSE events from the daemon socket.
///
/// # Errors
///
/// Returns a message when the socket cannot be read or a `data:` line is not JSON.
pub fn read_events(
    socket: &Path,
    last_event_id: Option<u64>,
    limit: usize,
    timeout: Duration,
) -> Result<Vec<StreamEvent>, String> {
    let mut stream = UnixStream::connect(socket).map_err(|err| err.to_string())?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|err| err.to_string())?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|err| err.to_string())?;
    let query = last_event_id.map_or(String::new(), |id| format!("?last_event_id={id}"));
    let request = format!(
        "GET /api/events{query} HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|err| err.to_string())?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    let mut events = Vec::new();
    loop {
        if events.len() >= limit {
            break;
        }
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(chunk.get(..n).unwrap_or(&[]));
                events = parse_data(&buf)?;
                if events.len() >= limit {
                    events.truncate(limit);
                    break;
                }
            }
            Err(err)
                if err.kind() == std::io::ErrorKind::WouldBlock
                    || err.kind() == std::io::ErrorKind::TimedOut =>
            {
                break;
            }
            Err(err) => return Err(err.to_string()),
        }
    }
    Ok(events)
}

fn parse_data(buf: &[u8]) -> Result<Vec<StreamEvent>, String> {
    let text = String::from_utf8_lossy(buf);
    let mut events = Vec::new();
    for line in text.lines() {
        let Some(payload) = line.strip_prefix("data:") else {
            continue;
        };
        let payload = payload.trim();
        if payload.is_empty() {
            continue;
        }
        let event = serde_json::from_str(payload).map_err(|err| err.to_string())?;
        events.push(event);
    }
    Ok(events)
}
