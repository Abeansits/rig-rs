//! `rig workflow`. The daemon is the only writer. This module does not paste.

use std::path::Path;

use clap::{Args, Subcommand};
use serde_json::{Value, json};

use crate::autostart::ensure_daemon;
use crate::error::Error;
use crate::output;

const EXITS: &[&str] = &["handoff", "waiting", "done", "failed"];

/// `rig workflow`.
#[derive(Debug, Args)]
#[command(
    about = "Declarative workflow runtime",
    long_about = "Declarative workflow runtime. run runs, watch watches, neither advances."
)]
pub(crate) struct WorkflowArgs {
    #[command(subcommand)]
    command: WorkflowCmd,
}

#[derive(Debug, Subcommand)]
enum WorkflowCmd {
    /// Validate a workflow spec file.
    Validate {
        /// Spec path.
        spec: String,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// Create an instance and its entry packet.
    Instantiate {
        /// Spec path.
        spec: String,
        /// Root objective.
        #[arg(long)]
        root_objective: String,
        /// Creator session.
        #[arg(long)]
        created_by: String,
        /// Entry owner override.
        #[arg(long)]
        entry_owner: Option<String>,
        /// Bind this rig.
        #[arg(long)]
        rig: Option<String>,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// Close the frontier packet and project the successor.
    Project {
        /// Instance id.
        #[arg(long)]
        instance: String,
        /// Frontier packet.
        #[arg(long)]
        current_packet: String,
        /// handoff, waiting, done, or failed.
        #[arg(long)]
        exit: String,
        /// Actor session.
        #[arg(long)]
        actor_session: String,
        /// Closure note.
        #[arg(long)]
        result_note: Option<String>,
        /// Waiting blocker.
        #[arg(long)]
        blocked_on: Option<String>,
        /// Next owner override.
        #[arg(long)]
        next_owner: Option<String>,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// List instances.
    List {
        /// Filter by status.
        #[arg(long)]
        status: Option<String>,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// List cached specs.
    Specs {
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show one instance.
    Show {
        /// Instance id.
        id: String,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// Instance plus trail.
    Trace {
        /// Instance id.
        id: String,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// Read-only inspector.
    Continue {
        /// Instance id.
        id: String,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// Current-step guidance.
    Guidance {
        /// Instance id.
        id: String,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// Retarget the frontier owner. Does not advance the step.
    Route {
        /// Instance id.
        id: String,
        /// New owner.
        #[arg(long)]
        to: String,
        /// Actor session.
        #[arg(long)]
        actor_session: String,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// Redrive a failed or aborted instance.
    Resume {
        /// Instance id.
        id: String,
        /// Actor session.
        #[arg(long)]
        actor_session: String,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// Cancel live packets and abort the instance.
    Abort {
        /// Instance id.
        id: String,
        /// Why.
        #[arg(long)]
        reason: String,
        /// Actor session.
        #[arg(long)]
        actor_session: String,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// Instantiate and read the outcome. Does not advance a step.
    Run {
        /// Spec path.
        spec: String,
        /// Root objective.
        #[arg(long)]
        root_objective: String,
        /// Creator session.
        #[arg(long)]
        created_by: String,
        /// Entry owner override.
        #[arg(long)]
        entry_owner: Option<String>,
        /// Bind this rig.
        #[arg(long)]
        rig: Option<String>,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
    /// Read an instance. Does not advance a step.
    Watch {
        /// Instance id.
        id: String,
        /// Print the daemon JSON.
        #[arg(long)]
        json: bool,
    },
}

pub(crate) fn run(args: &WorkflowArgs) -> Result<i32, Error> {
    match &args.command {
        WorkflowCmd::Validate { spec, json } => validate_spec(spec, *json),
        WorkflowCmd::Instantiate { .. } => {
            instantiate(&args.command, Some(SummaryKind::Instantiate))
        }
        WorkflowCmd::Project { .. } => project(&args.command),
        WorkflowCmd::List { status, json } => list(status.as_deref(), *json),
        WorkflowCmd::Specs { json } => get("/api/workflow/specs", *json),
        WorkflowCmd::Show { id, json } => get(&format!("/api/workflow/{id}"), *json),
        WorkflowCmd::Trace { id, json } => get(&format!("/api/workflow/{id}/trace"), *json),
        WorkflowCmd::Continue { id, json } => post(
            &format!("/api/workflow/{id}/continue"),
            &json!({}),
            *json,
            None,
        ),
        WorkflowCmd::Guidance { id, json } => get(&format!("/api/workflow/{id}/guidance"), *json),
        WorkflowCmd::Route {
            id,
            to,
            actor_session,
            json,
        } => post(
            &format!("/api/workflow/{id}/route"),
            &json!({"toSession": to, "actorSession": actor_session}),
            *json,
            Some(SummaryKind::Route),
        ),
        WorkflowCmd::Resume {
            id,
            actor_session,
            json,
        } => post(
            &format!("/api/workflow/{id}/resume"),
            &json!({"actorSession": actor_session}),
            *json,
            Some(SummaryKind::Resume),
        ),
        WorkflowCmd::Abort {
            id,
            reason,
            actor_session,
            json,
        } => post(
            &format!("/api/workflow/{id}/abort"),
            &json!({"reason": reason, "actorSession": actor_session}),
            *json,
            Some(SummaryKind::Abort),
        ),
        WorkflowCmd::Run { .. } => instantiate(&args.command, None),
        WorkflowCmd::Watch { id, json } => watch(id, *json),
    }
}

fn validate_spec(spec: &str, json: bool) -> Result<i32, Error> {
    let path = absolute(spec)?;
    post(
        "/api/workflow/validate",
        &json!({"specPath": path}),
        json,
        None,
    )
}

fn instantiate(cmd: &WorkflowCmd, summary: Option<SummaryKind>) -> Result<i32, Error> {
    let (WorkflowCmd::Instantiate {
        spec,
        root_objective,
        created_by,
        entry_owner,
        rig,
        json,
    }
    | WorkflowCmd::Run {
        spec,
        root_objective,
        created_by,
        entry_owner,
        rig,
        json,
    }) = cmd
    else {
        return Ok(0);
    };
    post(
        "/api/workflow/instantiate",
        &json!({
            "specPath": absolute(spec)?,
            "rootObjective": root_objective,
            "createdBySession": created_by,
            "entryOwnerSession": entry_owner,
            "targetRig": rig,
        }),
        *json,
        summary,
    )
}

fn project(cmd: &WorkflowCmd) -> Result<i32, Error> {
    let WorkflowCmd::Project {
        instance,
        current_packet,
        exit,
        actor_session,
        result_note,
        blocked_on,
        next_owner,
        json,
    } = cmd
    else {
        return Ok(0);
    };
    if !EXITS.contains(&exit.as_str()) {
        return Ok(bad_exit(exit, *json));
    }
    post(
        "/api/workflow/project",
        &json!({
            "instanceId": instance,
            "currentPacketId": current_packet,
            "exit": exit,
            "actorSession": actor_session,
            "resultNote": result_note,
            "blockedOn": blocked_on,
            "nextOwnerSession": next_owner,
        }),
        *json,
        Some(SummaryKind::Project),
    )
}

fn list(status: Option<&str>, json: bool) -> Result<i32, Error> {
    let path = match status {
        Some(status) => format!("/api/workflow/list?status={status}"),
        None => "/api/workflow/list".to_owned(),
    };
    get(&path, json)
}

#[derive(Clone, Copy)]
enum SummaryKind {
    Instantiate,
    Project,
    Route,
    Resume,
    Abort,
}

fn absolute(spec: &str) -> Result<String, Error> {
    let path = Path::new(spec);
    let full = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(full.display().to_string())
}

fn bad_exit(exit: &str, json: bool) -> i32 {
    let fact = format!(
        "--exit must be one of {} (got \"{exit}\").",
        EXITS.join(" | ")
    );
    let body = json!({
        "ok": false,
        "error": {
            "fact": fact,
            "consequence": "rig workflow project did not run; daemon was not contacted.",
            "action": "Pass a valid exit kind. Example: --exit handoff.",
        }
    });
    if json {
        output::stdout_json(&body);
    } else {
        output::stderr_line(&format!(
            "Error: {fact}\nrig workflow project did not run; daemon was not contacted.\nPass a valid exit kind. Example: --exit handoff."
        ));
    }
    1
}

fn post(path: &str, body: &Value, json: bool, summary: Option<SummaryKind>) -> Result<i32, Error> {
    let client = ensure_daemon()?;
    let bytes = serde_json::to_vec(body).map_err(|err| Error::Json(err.to_string()))?;
    let response = client
        .request("POST", path, Some(&bytes))
        .map_err(|err| Error::Unreachable(err.to_string()))?;
    finish(response.status, &response.body, json, summary)
}

fn get(path: &str, json: bool) -> Result<i32, Error> {
    let client = ensure_daemon()?;
    let response = client
        .request("GET", path, None)
        .map_err(|err| Error::Unreachable(err.to_string()))?;
    finish(response.status, &response.body, json, None)
}

fn watch(id: &str, json: bool) -> Result<i32, Error> {
    let client = ensure_daemon()?;
    let path = format!("/api/workflow/{id}");
    let response = client
        .request("GET", &path, None)
        .map_err(|err| Error::Unreachable(err.to_string()))?;
    let code = finish(response.status, &response.body, json, None)?;
    if code == 0 {
        Ok(outcome_code(&response.body))
    } else {
        Ok(code)
    }
}

fn finish(status: u16, body: &str, json: bool, summary: Option<SummaryKind>) -> Result<i32, Error> {
    if !(200..300).contains(&status) {
        let message = serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|value| {
                value
                    .get("message")
                    .or_else(|| value.get("error"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| body.to_owned());
        output::stderr_line(&message);
        return Ok(if status >= 500 { 2 } else { 1 });
    }
    let value = serde_json::from_str::<Value>(body).map_err(|err| Error::Json(err.to_string()))?;
    advisories(&value);
    if json {
        output::stdout_json(&value);
    } else {
        output::stdout_json(&value);
        if let Some(kind) = summary {
            output::stdout_line("");
            output::stdout_line(&format!("  what:  {}", what(&value, kind)));
            output::stdout_line(&format!("  state: {}", state_line(&value)));
            output::stdout_line(&format!("  next:  {}", next_line(&value)));
        }
    }
    Ok(0)
}

fn advisories(value: &Value) {
    let Some(rows) = value.get("advisories").and_then(Value::as_array) else {
        return;
    };
    for row in rows {
        if let Some(line) = row.as_str() {
            output::stderr_line(&format!("⚠ workflow advisory: {line}"));
        }
    }
}

fn what(value: &Value, kind: SummaryKind) -> String {
    match kind {
        SummaryKind::Instantiate => format!(
            "Instantiated workflow (instance {})",
            text(value, "/instance/instanceId")
        ),
        SummaryKind::Project => format!(
            "Closed {} ({})",
            text(value, "/closurePriorPacketId"),
            text(value, "/closureReason")
        ),
        SummaryKind::Route => format!(
            "Re-routed step {} to {}",
            text(value, "/stepId"),
            text(value, "/toSession")
        ),
        SummaryKind::Resume => format!("Redrove packet {}", text(value, "/newPacketId")),
        SummaryKind::Abort => "Aborted workflow".to_owned(),
    }
}

fn state_line(value: &Value) -> String {
    let status = value
        .pointer("/instance/status")
        .or_else(|| value.pointer("/instanceStatus"))
        .or_else(|| value.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("active");
    format!("status {status}")
}

fn next_line(value: &Value) -> String {
    value
        .pointer("/nextQitemId")
        .or_else(|| value.get("newPacketId"))
        .and_then(Value::as_str)
        .map_or_else(
            || "rig workflow show".to_owned(),
            |id| format!("rig queue show {id}"),
        )
}

fn text(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or("-")
        .to_owned()
}

fn outcome_code(body: &str) -> i32 {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return 0;
    };
    let status = value
        .get("status")
        .or_else(|| value.pointer("/instance/status"))
        .and_then(Value::as_str);
    match status {
        Some("failed" | "aborted") => 3,
        _ => 0,
    }
}
