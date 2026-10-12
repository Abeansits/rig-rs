//! `rig compact <seat>`.

use clap::Args;
use crewmux_api::{CompactCancel, CompactStatus, CompactTrigger, Endpoint};
use serde_json::{Value, json};

use crate::autostart::ensure_daemon;
use crate::error::Error;
use crate::output;

/// Compact one Claude seat, or show and cancel an attempt.
#[derive(Debug, Args)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each switch is a separate clap flag"
)]
pub(crate) struct CompactArgs {
    /// Seat name.
    pub(crate) seat: String,
    /// End the open managed attempt.
    #[arg(long)]
    pub(crate) cancel: bool,
    /// Print attempt state, including summary sha256 and length.
    #[arg(long)]
    pub(crate) status: bool,
    /// Compact once without waiting for a restore map.
    #[arg(long)]
    pub(crate) skip_map: bool,
    /// Print the daemon JSON.
    #[arg(long)]
    pub(crate) json: bool,
}

pub(crate) fn run(args: &CompactArgs) -> Result<i32, Error> {
    let client = ensure_daemon()?;
    let (method, path, body) = request(args)?;
    let response = client
        .request(method, path, Some(&body))
        .map_err(|err| Error::Unreachable(err.to_string()))?;
    if response.status != 200 {
        return http_error(&response.body);
    }
    let value = serde_json::from_str::<Value>(&response.body)
        .map_err(|err| Error::Json(err.to_string()))?;
    if args.json {
        output::stdout_json(&value);
    } else {
        print_status(&value);
    }
    Ok(0)
}

fn request(args: &CompactArgs) -> Result<(&'static str, &'static str, Vec<u8>), Error> {
    let value = if args.status || args.cancel {
        json!({"seat": args.seat})
    } else {
        json!({"seat": args.seat, "skip_map": args.skip_map})
    };
    let body = serde_json::to_vec(&value).map_err(|err| Error::Json(err.to_string()))?;
    let (method, path) = if args.status {
        (CompactStatus::METHOD, CompactStatus::PATH)
    } else if args.cancel {
        (CompactCancel::METHOD, CompactCancel::PATH)
    } else {
        (CompactTrigger::METHOD, CompactTrigger::PATH)
    };
    Ok((method, path, body))
}

fn print_status(value: &Value) {
    let stage = text(value, "stage");
    output::stdout_line(&format!("stage\t{stage}"));
    let Some(attempts) = value.get("attempts").and_then(Value::as_array) else {
        return;
    };
    for attempt in attempts {
        let phase = text(attempt, "state");
        let id = text(attempt, "attempt_id");
        let sha = text(attempt, "summary_sha");
        let len = text(attempt, "summary_len");
        let reason = text(attempt, "abort_reason");
        output::stdout_line(&format!(
            "{phase}\t{id}\tsha={sha}\tlen={len}\treason={reason}"
        ));
    }
}

fn text(value: &Value, key: &str) -> String {
    match value.get(key) {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(flag)) => flag.to_string(),
        _ => "-".to_owned(),
    }
}

fn http_error(body: &str) -> Result<i32, Error> {
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
