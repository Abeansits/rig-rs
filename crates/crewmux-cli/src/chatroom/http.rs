//! Chat HTTP calls and the human-readable lines.

use std::fmt::Write as _;
use std::io::ErrorKind;
use std::thread;
use std::time::Duration;

use crewmux_api::{ChatHistory, ChatSend, Client, Endpoint, RigSummary};
use crewmux_types::{SEAT_ID_ENV, SEAT_ID_HEADER, SEAT_TOKEN_ENV, SEAT_TOKEN_HEADER};
use serde_json::Value;

use crate::autostart::ensure_daemon;
use crate::error::Error;
use crate::output;

use super::resolve_name;

pub(super) fn send(rig: &str, message: &str, json: bool) -> Result<i32, Error> {
    let client = ensure_daemon()?;
    let id = rig_id(&client, rig)?;
    let path = format!("/api/rigs/{id}/chat/send");
    let payload = serde_json::to_vec(&serde_json::json!({"body": message}))
        .map_err(|err| Error::Json(err.to_string()))?;
    let response = post(&client, &path, &payload)?;
    if response.status != 201 {
        return fail(response.status, &response.body, false);
    }
    if json {
        output::stdout_line(response.body.trim());
        return Ok(0);
    }
    let value = parse(&response.body)?;
    output::stdout_line(&format_message(&value));
    Ok(0)
}

pub(super) struct HistoryRequest<'a> {
    pub(super) rig: &'a str,
    pub(super) topic: Option<&'a str>,
    pub(super) after: Option<&'a str>,
    pub(super) since: Option<&'a str>,
    pub(super) sender: Option<&'a str>,
    pub(super) limit: u32,
    pub(super) json: bool,
}

pub(super) fn history(request: &HistoryRequest<'_>) -> Result<i32, Error> {
    let client = ensure_daemon()?;
    let id = rig_id(&client, request.rig)?;
    let mut path = format!("/api/rigs/{id}/chat/history");
    push_query(&mut path, "limit", &request.limit.to_string());
    if let Some(topic) = request.topic {
        push_query(&mut path, "topic", topic);
    }
    if let Some(after) = request.after {
        push_query(&mut path, "after", after);
    }
    if let Some(since) = request.since {
        push_query(&mut path, "since", since);
    }
    if let Some(sender) = request.sender {
        push_query(&mut path, "sender", sender);
    }
    let response = call(&client, ChatHistory::METHOD, &path, None, &[])?;
    if response.status != 200 {
        return fail(response.status, &response.body, request.json);
    }
    if request.json {
        output::stdout_line(response.body.trim());
        return Ok(0);
    }
    let value = parse(&response.body)?;
    let Some(rows) = value.as_array() else {
        return Err(Error::Json("history was not an array".to_owned()));
    };
    if rows.is_empty() {
        output::stdout_line("No messages.");
        return Ok(0);
    }
    for row in rows {
        output::stdout_line(&format_message(row));
    }
    Ok(0)
}

pub(super) fn topic(rig: &str, name: &str, body: Option<&str>, json: bool) -> Result<i32, Error> {
    let client = ensure_daemon()?;
    let id = rig_id(&client, rig)?;
    let path = format!("/api/rigs/{id}/chat/topic");
    let payload = serde_json::to_vec(&serde_json::json!({"topic": name, "body": body}))
        .map_err(|err| Error::Json(err.to_string()))?;
    let response = post(&client, &path, &payload)?;
    if response.status != 201 {
        return fail(response.status, &response.body, false);
    }
    if json {
        output::stdout_line(response.body.trim());
        return Ok(0);
    }
    output::stdout_line(&format!("--- topic: {name} ---"));
    Ok(0)
}

pub(super) fn clear(rig: &str, json: bool) -> Result<i32, Error> {
    let client = ensure_daemon()?;
    let id = rig_id(&client, rig)?;
    let path = format!("/api/rigs/{id}/chat/clear");
    let response = post(&client, &path, b"{}")?;
    if response.status != 200 {
        return fail(response.status, &response.body, json);
    }
    if json {
        output::stdout_line(response.body.trim());
        return Ok(0);
    }
    let value = parse(&response.body)?;
    let deleted = value.get("deleted").and_then(Value::as_u64).unwrap_or(0);
    output::stdout_line(&format!("Cleared {deleted} messages from {rig} chatroom."));
    Ok(0)
}

pub(super) fn rig_id(client: &Client, name: &str) -> Result<String, Error> {
    let response = call(client, RigSummary::METHOD, RigSummary::PATH, None, &[])?;
    if response.status != 200 {
        return match fail(response.status, &response.body, false) {
            Err(err) => Err(err),
            Ok(_) => Err(Error::User(response.body)),
        };
    }
    let rows: Vec<crewmux_api::RigSummaryRow> =
        serde_json::from_str(&response.body).map_err(|err| Error::Json(err.to_string()))?;
    resolve_name(&rows, name)
        .map(str::to_owned)
        .map_err(Error::User)
}

pub(super) fn open_watch(
    client: &Client,
    rig: &str,
) -> Result<(String, crewmux_api::HttpStream), Error> {
    let id = rig_id(client, rig)?;
    let path = format!("/api/rigs/{id}/chat/watch");
    let mut last = String::new();
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            thread::park_timeout(RETRY_GAP);
        }
        match client.open("GET", &path, &[]) {
            Ok(stream) => return Ok((path, stream)),
            Err(err) if transient(&err) => last = err.to_string(),
            Err(err) => return Err(Error::Unreachable(err.to_string())),
        }
    }
    Err(Error::Unreachable(last))
}

fn post(client: &Client, path: &str, body: &[u8]) -> Result<crewmux_api::Response, Error> {
    let headers = seat_headers();
    let view: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    call(client, ChatSend::METHOD, path, Some(body), &view)
}

const ATTEMPTS: u32 = 8;
const RETRY_GAP: Duration = Duration::from_millis(20);

fn call(
    client: &Client,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
    headers: &[(&str, &str)],
) -> Result<crewmux_api::Response, Error> {
    let mut last = String::new();
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            thread::park_timeout(RETRY_GAP);
        }
        match client.request_with_headers(method, path, body, headers) {
            Ok(response) => return Ok(response),
            Err(err) if method == "GET" && transient(&err) => last = err.to_string(),
            Err(err) => return Err(Error::Unreachable(err.to_string())),
        }
    }
    Err(Error::Unreachable(last))
}

fn transient(err: &crewmux_api::Error) -> bool {
    let (crewmux_api::Error::Connect { source, .. } | crewmux_api::Error::Exchange { source, .. }) =
        err
    else {
        return false;
    };
    matches!(
        source.kind(),
        ErrorKind::ConnectionReset
            | ErrorKind::ConnectionRefused
            | ErrorKind::BrokenPipe
            | ErrorKind::TimedOut
            | ErrorKind::Interrupted
    )
}

fn seat_headers() -> Vec<(&'static str, String)> {
    let id = std::env::var(SEAT_ID_ENV).ok();
    let token = std::env::var(SEAT_TOKEN_ENV).ok();
    match (id, token) {
        (None, None) => Vec::new(),
        (Some(id), Some(token)) => vec![(SEAT_ID_HEADER, id), (SEAT_TOKEN_HEADER, token)],
        (Some(id), None) => vec![(SEAT_ID_HEADER, id), (SEAT_TOKEN_HEADER, String::new())],
        (None, Some(token)) => vec![(SEAT_ID_HEADER, String::new()), (SEAT_TOKEN_HEADER, token)],
    }
}

fn push_query(path: &mut String, key: &str, value: &str) {
    path.push(if path.contains('?') { '&' } else { '?' });
    path.push_str(key);
    path.push('=');
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            path.push(char::from(byte));
        } else {
            let _ = write!(path, "%{byte:02X}");
        }
    }
}

pub(super) fn format_message(value: &Value) -> String {
    if value.get("kind").and_then(Value::as_str) == Some("topic") {
        let topic = value.get("topic").and_then(Value::as_str).unwrap_or("");
        return format!("--- topic: {topic} ---");
    }
    let sender = value
        .get("sender")
        .and_then(Value::as_str)
        .unwrap_or("operator");
    let body = value.get("body").and_then(Value::as_str).unwrap_or("");
    format!("[{sender}] {body}")
}

fn parse(body: &str) -> Result<Value, Error> {
    serde_json::from_str(body).map_err(|err| Error::Json(err.to_string()))
}

fn fail(status: u16, body: &str, json_stdout: bool) -> Result<i32, Error> {
    if json_stdout {
        output::stdout_line(body.trim());
        return Ok(1);
    }
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| {
            if body.trim().is_empty() {
                format!("HTTP {status}")
            } else {
                body.trim().to_owned()
            }
        });
    Err(Error::User(message))
}
