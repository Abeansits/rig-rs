//! `rig` verbs. State changes go through the daemon; this crate does not link it.

mod autostart;
mod error;
pub mod output;

pub use error::Error;

use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use clap::{Parser, Subcommand};
use crewmux_api::{AttachInfo, CaptureSeat, CreateRigUp, Endpoint, ListSeats, RigDown};
use serde_json::Value;

use crate::autostart::ensure_daemon;

/// Parsed argv for every verb except `daemon`, which the binary dispatches itself.
#[derive(Debug, Parser)]
#[command(name = "rig", version = "0.0.0", about = "Run stub seats")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Launch a rig from a YAML spec.
    Up {
        /// Spec path.
        spec: PathBuf,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// List rigs and seats.
    Ps {
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print a seat's screen.
    Capture {
        /// Seat name.
        seat: String,
        /// Return only the last N lines.
        #[arg(long)]
        lines: Option<u32>,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// Exec `tmux attach` for a seat.
    Attach {
        /// Seat name.
        seat: String,
    },
    /// Stop one rig, or every rig when the name is omitted.
    Down {
        /// Rig name.
        rig: Option<String>,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
}

/// Run one CLI invocation. The process exit code is the `Ok` value, or [`Error::code`].
///
/// # Errors
///
/// Returns [`Error`] for a user mistake, a missing daemon, or I/O.
pub fn run() -> Result<i32, Error> {
    match Cli::try_parse() {
        Ok(cli) => dispatch(cli),
        Err(err) => {
            use clap::error::ErrorKind;
            if matches!(
                err.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) {
                output::stdout_line(err.to_string().trim_end());
                Ok(0)
            } else {
                output::stderr_line(&err.to_string());
                Ok(1)
            }
        }
    }
}

fn dispatch(cli: Cli) -> Result<i32, Error> {
    match cli.command {
        Commands::Up { spec, json } => up(&spec, json),
        Commands::Ps { json } => ps(json),
        Commands::Capture { seat, lines, json } => capture(&seat, lines, json),
        Commands::Attach { seat } => attach(&seat),
        Commands::Down { rig, json } => down(rig.as_deref(), json),
    }
}

fn up(spec: &std::path::Path, json: bool) -> Result<i32, Error> {
    let text = std::fs::read_to_string(spec)
        .map_err(|err| Error::User(format!("failed to read {}: {err}", spec.display())))?;
    crewmux_spec::parse(&text).map_err(|err| Error::User(err.to_string()))?;
    let client = ensure_daemon()?;
    let response = client
        .request(
            CreateRigUp::METHOD,
            CreateRigUp::PATH,
            Some(text.as_bytes()),
        )
        .map_err(|err| Error::Unreachable(err.to_string()))?;
    finish(response.status, &response.body, json)
}

fn ps(json: bool) -> Result<i32, Error> {
    let client = ensure_daemon()?;
    let response = client
        .request(ListSeats::METHOD, ListSeats::PATH, None)
        .map_err(|err| Error::Unreachable(err.to_string()))?;
    finish(response.status, &response.body, json)
}

fn capture(seat: &str, lines: Option<u32>, json: bool) -> Result<i32, Error> {
    let client = ensure_daemon()?;
    let body = serde_json::to_vec(&serde_json::json!({"seat": seat, "lines": lines}))
        .map_err(|err| Error::Json(err.to_string()))?;
    let response = client
        .request(CaptureSeat::METHOD, CaptureSeat::PATH, Some(&body))
        .map_err(|err| Error::Unreachable(err.to_string()))?;
    if response.status != 200 {
        return http_error(response.status, &response.body);
    }
    let value = parse_json(&response.body)?;
    if json {
        output::stdout_json(&value);
    } else {
        output::stdout_capture(&value);
    }
    Ok(0)
}

fn attach(seat: &str) -> Result<i32, Error> {
    let client = ensure_daemon()?;
    let body = serde_json::to_vec(&serde_json::json!({"seat": seat}))
        .map_err(|err| Error::Json(err.to_string()))?;
    let response = client
        .with_timeout(Duration::from_secs(5))
        .request(AttachInfo::METHOD, AttachInfo::PATH, Some(&body))
        .map_err(|err| Error::Unreachable(err.to_string()))?;
    if response.status != 200 {
        return http_error(response.status, &response.body);
    }
    let value = parse_json(&response.body)?;
    let argv = value
        .get("argv")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Json("attach response has no argv".to_owned()))?;
    let mut parts = Vec::new();
    for item in argv {
        let Some(text) = item.as_str() else {
            return Err(Error::Json("attach argv must be strings".to_owned()));
        };
        parts.push(text.to_owned());
    }
    let Some(program) = parts.first() else {
        return Err(Error::Json("attach argv is empty".to_owned()));
    };
    let error = Command::new(program).args(parts.iter().skip(1)).exec();
    Err(Error::User(format!("failed to exec tmux: {error}")))
}

fn down(rig: Option<&str>, json: bool) -> Result<i32, Error> {
    let client = ensure_daemon()?;
    let payload = serde_json::json!({"rig": rig});
    let body = serde_json::to_vec(&payload).map_err(|err| Error::Json(err.to_string()))?;
    let response = client
        .request(RigDown::METHOD, RigDown::PATH, Some(&body))
        .map_err(|err| Error::Unreachable(err.to_string()))?;
    finish(response.status, &response.body, json)
}

fn finish(status: u16, body: &str, json: bool) -> Result<i32, Error> {
    if status != 200 {
        return http_error(status, body);
    }
    let value = parse_json(body)?;
    if json {
        output::stdout_json(&value);
    } else {
        output::stdout_seats(&value);
    }
    let code = value
        .get("status")
        .and_then(Value::as_str)
        .map_or(0, |status| if status == "partial" { 3 } else { 0 });
    Ok(code)
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
