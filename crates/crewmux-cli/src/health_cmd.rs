// Derived from openrig packages/cli/src/commands/health.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! `rig health`, `rig stream`, and `rig usage`.

use std::io::Read;
use std::time::Duration;

use clap::{Args, Subcommand};
use serde_json::Value;

use crate::autostart::ensure_daemon;
use crate::error::Error;
use crate::output;

/// `rig health`.
#[derive(Debug, Args)]
pub(crate) struct HealthArgs {
    /// Print the daemon JSON.
    #[arg(long)]
    json: bool,
    /// Page size, 1 to 200.
    #[arg(long)]
    limit: Option<u32>,
    #[command(subcommand)]
    command: Option<HealthCmd>,
}

#[derive(Debug, Subcommand)]
enum HealthCmd {
    /// Print one finding.
    Explain {
        /// Finding id.
        id: String,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
}

/// `rig stream`.
#[derive(Debug, Args)]
pub(crate) struct StreamArgs {
    #[command(subcommand)]
    command: StreamCmd,
}

#[derive(Debug, Subcommand)]
enum StreamCmd {
    /// Append a stream item.
    Emit {
        /// Body text.
        #[arg(long)]
        body: String,
        /// Stable id. A repeat emit returns the existing item.
        #[arg(long)]
        id: Option<String>,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// List active items, oldest first.
    List {
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print one item.
    Show {
        /// Stream item id.
        id: String,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// Archive one item.
    Archive {
        /// Stream item id.
        id: String,
    },
    /// Follow `/api/events` for stream items. Resume with `--last-event-id`.
    Watch {
        /// Hub id already seen.
        #[arg(long)]
        last_event_id: Option<u64>,
        /// Print the backlog and return.
        #[arg(long)]
        once: bool,
    },
    /// Operator reclaim of the classifier lease.
    ReclaimClassifier {
        /// Refuse while the holder is still inside the TTL.
        #[arg(long)]
        if_dead: bool,
    },
}

/// `rig usage`.
#[derive(Debug, Args)]
pub(crate) struct UsageArgs {
    /// Print the daemon JSON.
    #[arg(long)]
    json: bool,
}

pub(crate) fn health(args: &HealthArgs) -> Result<i32, Error> {
    if let Some(HealthCmd::Explain { id, json }) = &args.command {
        return call("GET", &format!("/api/health/{id}"), None, *json);
    }
    let limit = args.limit.unwrap_or(100);
    call(
        "GET",
        &format!("/api/health?limit={limit}"),
        None,
        args.json,
    )
}

pub(crate) fn stream(args: &StreamArgs) -> Result<i32, Error> {
    match &args.command {
        StreamCmd::Emit { body, id, json } => {
            let payload = serde_json::json!({"body": body, "id": id, "kind": "note"});
            call("POST", "/api/stream/emit", Some(&payload), *json)
        }
        StreamCmd::List { json } => call("GET", "/api/stream/list", None, *json),
        StreamCmd::Show { id, json } => call("GET", &format!("/api/stream/{id}"), None, *json),
        StreamCmd::Archive { id } => call(
            "POST",
            &format!("/api/stream/{id}/archive"),
            Some(&serde_json::json!({})),
            true,
        ),
        StreamCmd::Watch {
            last_event_id,
            once,
        } => watch(*last_event_id, *once),
        StreamCmd::ReclaimClassifier { if_dead } => {
            let payload = serde_json::json!({"ifDead": if_dead});
            call(
                "POST",
                "/api/stream/reclaim-classifier",
                Some(&payload),
                true,
            )
        }
    }
}

pub(crate) fn usage(args: &UsageArgs) -> Result<i32, Error> {
    call("GET", "/api/usage", None, args.json)
}

fn watch(last_event_id: Option<u64>, once: bool) -> Result<i32, Error> {
    let client = ensure_daemon()?;
    let path = match last_event_id {
        Some(id) => format!("/api/stream/watch?last_event_id={id}"),
        None => "/api/stream/watch".to_owned(),
    };
    let mut stream = client
        .open("GET", &path, &[])
        .map_err(|err| Error::Unreachable(err.to_string()))?;
    if stream.status != 200 {
        let mut body = String::new();
        drop(stream.read_to_string(&mut body));
        return http_error(stream.status, &body);
    }
    let timeout = if once {
        Some(Duration::from_millis(200))
    } else {
        Some(Duration::from_secs(3600))
    };
    stream
        .set_read_timeout(timeout)
        .map_err(|err| Error::User(err.to_string()))?;
    let mut buf = [0_u8; 2048];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let Some(chunk) = buf.get(..n) else {
                    break;
                };
                let text = String::from_utf8_lossy(chunk);
                output::stdout_line(text.trim_end());
                if once {
                    break;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::TimedOut && once => break,
            Err(err) => return Err(Error::User(err.to_string())),
        }
    }
    Ok(0)
}

fn call(method: &str, path: &str, body: Option<&Value>, json: bool) -> Result<i32, Error> {
    let client = ensure_daemon()?;
    let bytes = match body {
        Some(value) => Some(serde_json::to_vec(value).map_err(|err| Error::Json(err.to_string()))?),
        None => None,
    };
    let response = client
        .request(method, path, bytes.as_deref())
        .map_err(|err| Error::Unreachable(err.to_string()))?;
    if response.status != 200 && response.status != 201 {
        return http_error(response.status, &response.body);
    }
    let value = parse_json(&response.body)?;
    if json {
        output::stdout_json(&value);
    } else {
        print_text(&value);
    }
    Ok(0)
}

fn print_text(value: &Value) {
    if let Some(records) = value.get("records").and_then(Value::as_array) {
        if records.is_empty() {
            output::stdout_line("no findings");
            return;
        }
        for record in records {
            let detector = record.get("detector").and_then(Value::as_str).unwrap_or("");
            let summary = record.get("summary").and_then(Value::as_str).unwrap_or("");
            output::stdout_line(&format!("{detector} {summary}"));
        }
        return;
    }
    if let Some(text) = value.get("summary").and_then(Value::as_str) {
        output::stdout_line(text);
        return;
    }
    output::stdout_json(value);
}

fn parse_json(body: &str) -> Result<Value, Error> {
    serde_json::from_str(body).map_err(|err| Error::Json(err.to_string()))
}

fn http_error(status: u16, body: &str) -> Result<i32, Error> {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| body.to_owned());
    Err(Error::User(format!("{status} {message}")))
}
