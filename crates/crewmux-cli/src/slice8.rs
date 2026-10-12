//! `rig snapshot`, `rig restore`, `rig restore-check`, `rig crash-cart`, `rig checkpoint`.

use std::time::Duration;

use clap::{Args, Subcommand};
use crewmux_api::{Client, Endpoint, Healthz, daemon_socket, rig_home};
use serde_json::{Value, json};

use crate::autostart::ensure_daemon;
use crate::error::Error;
use crate::output;

/// `rig snapshot`.
#[derive(Debug, Args)]
pub(crate) struct SnapshotArgs {
    #[command(subcommand)]
    command: SnapshotCmd,
}

#[derive(Debug, Subcommand)]
enum SnapshotCmd {
    /// Capture a rig snapshot.
    Create {
        /// Rig name.
        #[arg(long)]
        rig: String,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// List snapshots for a rig.
    List {
        /// Rig name.
        #[arg(long)]
        rig: String,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show one snapshot.
    Show {
        /// Rig name.
        #[arg(long)]
        rig: String,
        /// Snapshot id.
        #[arg(long)]
        id: String,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
}

/// `rig restore <rig>`.
#[derive(Debug, Args)]
pub(crate) struct RestoreArgs {
    /// Rig name.
    rig: String,
    /// Snapshot id. Omit to restore the live database.
    #[arg(long)]
    snapshot: Option<String>,
    /// Requested posture. Restore will not widen a recorded floor or lock.
    #[arg(long)]
    posture: Option<String>,
    /// Print the daemon JSON.
    #[arg(long)]
    json: bool,
}

/// `rig restore-check`.
#[derive(Debug, Args)]
pub(crate) struct RestoreCheckArgs {
    /// Print the daemon JSON.
    #[arg(long)]
    json: bool,
}

/// `rig crash-cart`.
#[derive(Debug, Args)]
pub(crate) struct CrashCartArgs {
    #[command(subcommand)]
    command: CrashCmd,
}

#[derive(Debug, Subcommand)]
enum CrashCmd {
    /// Restore rigs, kernel first.
    Restore {
        /// Let the in-flight rig finish, then skip the rest.
        #[arg(long)]
        cancel: bool,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
}

/// `rig checkpoint`.
#[derive(Debug, Args)]
pub(crate) struct CheckpointArgs {
    #[command(subcommand)]
    command: CheckpointCmd,
}

#[derive(Debug, Subcommand)]
enum CheckpointCmd {
    /// Write a checkpoint.
    Write {
        /// Seat name.
        #[arg(long)]
        seat: String,
        /// Summary.
        #[arg(long)]
        summary: String,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show the latest checkpoint.
    Show {
        /// Seat name.
        #[arg(long)]
        seat: String,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
}

pub(crate) fn snapshot(args: &SnapshotArgs) -> Result<i32, Error> {
    match &args.command {
        SnapshotCmd::Create { rig, json } => {
            post("/api/snapshot/create", &json!({"rig": rig}), *json)
        }
        SnapshotCmd::List { rig, json } => post("/api/snapshot/list", &json!({"rig": rig}), *json),
        SnapshotCmd::Show { rig, id, json } => {
            post("/api/snapshot/show", &json!({"rig": rig, "id": id}), *json)
        }
    }
}

pub(crate) fn restore(args: &RestoreArgs) -> Result<i32, Error> {
    post(
        "/api/restore",
        &json!({"rig": args.rig, "snapshot": args.snapshot, "posture": args.posture}),
        args.json,
    )
}

pub(crate) fn restore_check(args: &RestoreCheckArgs) -> Result<i32, Error> {
    if !daemon_up() {
        let value = daemon_down_report();
        if args.json {
            output::stdout_json(&value);
        } else {
            output::stdout_line("daemon down");
        }
        return Ok(0);
    }
    let client = ensure_daemon()?;
    let response = client
        .request("GET", "/api/restore-check", None)
        .map_err(|err| Error::Unreachable(err.to_string()))?;
    finish(response.status, &response.body, args.json)
}

pub(crate) fn crash_cart(args: &CrashCartArgs) -> Result<i32, Error> {
    match &args.command {
        CrashCmd::Restore { cancel, json } => {
            post("/api/crash-cart/restore", &json!({"cancel": cancel}), *json)
        }
    }
}

pub(crate) fn checkpoint(args: &CheckpointArgs) -> Result<i32, Error> {
    match &args.command {
        CheckpointCmd::Write {
            seat,
            summary,
            json,
        } => post(
            "/api/checkpoint",
            &json!({"seat": seat, "summary": summary}),
            *json,
        ),
        CheckpointCmd::Show { seat, json } => {
            post("/api/checkpoint/show", &json!({"seat": seat}), *json)
        }
    }
}

fn post(path: &str, body: &Value, json: bool) -> Result<i32, Error> {
    let client = ensure_daemon()?;
    let bytes = serde_json::to_vec(body).map_err(|err| Error::Json(err.to_string()))?;
    let response = client
        .request("POST", path, Some(&bytes))
        .map_err(|err| Error::Unreachable(err.to_string()))?;
    finish(response.status, &response.body, json)
}

fn finish(status: u16, body: &str, json: bool) -> Result<i32, Error> {
    if status != 200 {
        return http_error(status, body);
    }
    let value = serde_json::from_str::<Value>(body).map_err(|err| Error::Json(err.to_string()))?;
    if json {
        output::stdout_json(&value);
    } else if let Some(id) = value.get("id").and_then(Value::as_str) {
        output::stdout_line(id);
    } else if let Some(outcome) = value.get("outcome").and_then(Value::as_str) {
        output::stdout_line(outcome);
    } else {
        output::stdout_json(&value);
    }
    Ok(0)
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
        .unwrap_or_else(|| format!("daemon returned {status}"));
    Err(Error::User(message))
}

fn daemon_up() -> bool {
    let socket = daemon_socket(&rig_home());
    Client::over_uds(socket)
        .with_timeout(Duration::from_millis(300))
        .request(Healthz::METHOD, Healthz::PATH, None)
        .is_ok_and(|response| response.status == 200)
}

fn daemon_down_report() -> Value {
    json!({
        "classes": {
            "ready": 0,
            "ready_with_caveats": 0,
            "not_ready": 0,
            "attention_required": 0,
            "unknown": 1
        },
        "seats": [],
        "remediations": [{"action": "start the daemon", "execution_safe": false}]
    })
}
