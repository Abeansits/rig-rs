//! `rig queue`, `rig send`, `rig ask`, `rig broadcast`, `rig unclaim`, and `rig whoami`.

use clap::{Args, Subcommand};
use crewmux_api::{
    AskSeat, BroadcastSend, Endpoint, QueueClaim, QueueCollection, QueueHandoff, QueueShow,
    SendMessage, UnclaimItem, Whoami,
};
use serde_json::{Value, json};

use crate::autostart::ensure_daemon;
use crate::error::Error;
use crate::output;

/// `rig queue`.
#[derive(Debug, Args)]
pub(crate) struct QueueArgs {
    #[command(subcommand)]
    command: QueueCmd,
}

#[derive(Debug, Subcommand)]
enum QueueCmd {
    /// List queue items.
    List {
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show one item and its transitions.
    Show {
        /// Queue item id.
        id: String,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// Create a pending item.
    Create {
        /// Author session.
        #[arg(long)]
        source: String,
        /// Destination session.
        #[arg(long)]
        destination: String,
        /// Obligation body.
        #[arg(long)]
        body: String,
        /// Optional id.
        #[arg(long)]
        id: Option<String>,
    },
    /// Claim an item for a session.
    Claim {
        /// Queue item id.
        id: String,
        /// Claimant session.
        #[arg(long)]
        by: String,
        /// Take the claim from someone else. Requires a reason.
        #[arg(long)]
        force: Option<String>,
    },
    /// Hand the item to another session.
    Handoff {
        /// Queue item id.
        id: String,
        /// Next destination.
        #[arg(long)]
        to: String,
        /// Actor session.
        #[arg(long)]
        actor: String,
    },
}

/// `rig send`.
#[derive(Debug, Args)]
pub(crate) struct SendArgs {
    /// Recipient seat or `member@rig`. Repeat with commas for several.
    #[arg(long)]
    pub to: Option<String>,
    /// Message body. Also accepted as a positional when `--to` is set.
    pub body: Option<String>,
    /// Sender session. Defaults to `operator@host`.
    #[arg(long)]
    pub from: Option<String>,
    /// Every seat in this rig.
    #[arg(long)]
    pub rig: Option<String>,
    /// Drive an interactive prompt. Requires `--reason`.
    #[arg(long)]
    pub dangerously_interact: bool,
    /// Why the prompt is being driven.
    #[arg(long)]
    pub reason: Option<String>,
    /// Require the payload to be visible in the pane.
    #[arg(long)]
    pub verify: bool,
    /// Print the daemon JSON.
    #[arg(long)]
    pub json: bool,
}

/// `rig broadcast`.
#[derive(Debug, Args)]
pub(crate) struct BroadcastArgs {
    /// Message body.
    pub body: String,
    /// Limit to one rig.
    #[arg(long)]
    pub rig: Option<String>,
    /// Sender session.
    #[arg(long)]
    pub from: Option<String>,
    /// Print the daemon JSON.
    #[arg(long)]
    pub json: bool,
}

/// `rig ask`.
#[derive(Debug, Args)]
pub(crate) struct AskArgs {
    /// Search text.
    pub text: String,
    /// Seat name. Searches only that seat's transcripts.
    #[arg(long)]
    pub seat: Option<String>,
    /// Provider session token. Searches only that JSONL.
    #[arg(long)]
    pub session: Option<String>,
    /// Rig name.
    #[arg(long)]
    pub rig: Option<String>,
    /// Print the daemon JSON.
    #[arg(long)]
    pub json: bool,
}

/// `rig unclaim`.
#[derive(Debug, Args)]
pub(crate) struct UnclaimArgs {
    /// Queue item id.
    pub id: String,
    /// Session releasing the claim.
    #[arg(long)]
    pub by: String,
    /// Why it is being released.
    #[arg(long)]
    pub reason: String,
}

pub(crate) fn queue(args: &QueueArgs) -> Result<i32, Error> {
    match &args.command {
        QueueCmd::List { json } => {
            call(QueueCollection::METHOD, QueueCollection::PATH, None, *json)
        }
        QueueCmd::Show { id, json } => {
            let body = json!({"id": id});
            call(QueueShow::METHOD, QueueShow::PATH, Some(&body), *json)
        }
        QueueCmd::Create {
            source,
            destination,
            body,
            id,
        } => {
            let payload =
                json!({"source": source, "destination": destination, "body": body, "id": id});
            call("POST", QueueCollection::PATH, Some(&payload), true)
        }
        QueueCmd::Claim { id, by, force } => {
            let payload = json!({"id": id, "by": by, "force": force});
            call(QueueClaim::METHOD, QueueClaim::PATH, Some(&payload), true)
        }
        QueueCmd::Handoff { id, to, actor } => {
            let payload = json!({"id": id, "to": to, "actor": actor});
            call(
                QueueHandoff::METHOD,
                QueueHandoff::PATH,
                Some(&payload),
                true,
            )
        }
    }
}

pub(crate) fn send(args: &SendArgs) -> Result<i32, Error> {
    let body = args
        .body
        .clone()
        .ok_or_else(|| Error::User("rig send needs a message body".to_owned()))?;
    let payload = json!({
        "to": args.to,
        "body": body,
        "from": args.from,
        "rig": args.rig,
        "dangerously_interact": args.dangerously_interact,
        "reason": args.reason,
        "verify": args.verify,
    });
    call(
        SendMessage::METHOD,
        SendMessage::PATH,
        Some(&payload),
        args.json,
    )
}

pub(crate) fn broadcast(args: &BroadcastArgs) -> Result<i32, Error> {
    let payload = json!({"body": args.body, "rig": args.rig, "from": args.from, "all": true});
    call(
        BroadcastSend::METHOD,
        BroadcastSend::PATH,
        Some(&payload),
        args.json,
    )
}

pub(crate) fn ask(args: &AskArgs) -> Result<i32, Error> {
    let payload = json!({
        "text": args.text,
        "seat": args.seat,
        "session": args.session,
        "rig": args.rig,
    });
    let client = ensure_daemon()?;
    let bytes = serde_json::to_vec(&payload).map_err(|err| Error::Json(err.to_string()))?;
    let response = client
        .request(AskSeat::METHOD, AskSeat::PATH, Some(&bytes))
        .map_err(|err| Error::Unreachable(err.to_string()))?;
    if response.status != 200 {
        return http_error(response.status, &response.body);
    }
    let value = parse_json(&response.body)?;
    if args.json {
        output::stdout_json(&value);
        return Ok(0);
    }
    if let Some(lines) = value.get("guidance").and_then(Value::as_array) {
        for line in lines {
            if let Some(text) = line.as_str() {
                output::stdout_line(text);
            }
        }
    }
    if let Some(hit) = value
        .get("l1")
        .and_then(|hit| hit.get("text"))
        .and_then(Value::as_str)
    {
        output::stdout_line(hit);
    }
    if let Some(hit) = value
        .get("l2")
        .and_then(|hit| hit.get("text"))
        .and_then(Value::as_str)
    {
        output::stdout_line(hit);
    }
    Ok(0)
}

pub(crate) fn unclaim(args: &UnclaimArgs) -> Result<i32, Error> {
    let payload = json!({"id": args.id, "by": args.by, "reason": args.reason});
    call(UnclaimItem::METHOD, UnclaimItem::PATH, Some(&payload), true)
}

pub(crate) fn whoami() -> Result<i32, Error> {
    call(Whoami::METHOD, Whoami::PATH, None, true)
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
    if response.status != 200 {
        return failed_http(response.status, &response.body, json);
    }
    let value = parse_json(&response.body)?;
    if json {
        output::stdout_json(&value);
    } else if let Some(items) = value.get("items").and_then(Value::as_array) {
        for item in items {
            let id = item.get("id").and_then(Value::as_str).unwrap_or("");
            let state = item.get("state").and_then(Value::as_str).unwrap_or("");
            output::stdout_line(&format!("{id}\t{state}"));
        }
    } else {
        output::stdout_json(&value);
    }
    Ok(exit_of(&value))
}

/// A non-200 body that still carries receipts is the failed send document.
fn failed_http(status: u16, body: &str, json: bool) -> Result<i32, Error> {
    if json
        && let Ok(value) = serde_json::from_str::<Value>(body)
        && value.get("receipts").is_some()
    {
        output::stdout_json(&value);
        return Ok(exit_of(&value));
    }
    http_error(status, body)
}

fn exit_of(value: &Value) -> i32 {
    value
        .get("status")
        .and_then(Value::as_str)
        .map_or(0, |status| match status {
            "ok" => 0,
            "partial" => 3,
            _ => 1,
        })
}

fn http_error(_status: u16, body: &str) -> Result<i32, Error> {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| body.to_owned());
    Err(Error::User(message))
}

fn parse_json(body: &str) -> Result<Value, Error> {
    serde_json::from_str(body).map_err(|err| Error::Json(err.to_string()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::exit_of;

    #[test]
    fn a_failed_receipt_exits_1() {
        let value = json!({"status": "failed", "receipts": [{"outcome": "failed"}]});
        assert_eq!(exit_of(&value), 1);
        assert!(value.get("receipts").is_some());
    }
}
