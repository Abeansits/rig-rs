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

/// Exit 0 once `/compact` was submitted.
///
/// Exit 4 means the preparation was accepted and `/compact` has not been
/// submitted yet. Exit 1 is a refusal or a failure, including `send_failed`
/// and a prep the transcript did not confirm. A later abort, including
/// `no_post_compact` after 120s, does not change an exit 0. `--status` and
/// `--cancel` report state and return 0 when the daemon accepts them.
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
    if args.status || args.cancel {
        return Ok(0);
    }
    Ok(compact_exit(&value))
}

/// 0 after `/compact` is submitted. 4 while the prep is still pending.
fn compact_exit(value: &Value) -> i32 {
    if value.get("compact_sent").and_then(Value::as_bool) == Some(true) {
        0
    } else {
        4
    }
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
        Some(Value::String(text)) if !text.is_empty() => text.clone(),
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::compact_exit;

    #[test]
    fn captured_preparing_body_is_exit_4() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/claude/2.1.287/pr18-run5/e01-compact-preparing.rig.json");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{err}"));
        let value: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|err| panic!("{err}"));
        assert_eq!(value["stage"], "preparing");
        assert_eq!(value["compact_sent"], false);
        assert_eq!(compact_exit(&value), 4);
    }

    #[test]
    fn exit_is_zero_only_after_compact_is_submitted() {
        assert_eq!(compact_exit(&json!({"compact_sent": true})), 0);
        assert_eq!(
            compact_exit(&json!({"compact_sent": false, "stage": "preparing"})),
            4
        );
        assert_eq!(
            compact_exit(&json!({"stage": "waiting_map", "reason": "preparation_pending"})),
            4
        );
    }
}
