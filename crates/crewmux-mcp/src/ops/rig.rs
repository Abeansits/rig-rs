// Derived from openrig packages/cli/src/mcp-server.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! `rig_start`, `rig_stop`, and `rig_list`.

use std::path::Path;
use std::time::Duration;

use rmcp::model::CallToolResult;
use serde_json::{Value, json};

use crate::args::{NoArgs, RigStartArgs, RigStopArgs};
use crate::error::Error;
use crate::exchange::{Ctx, Request, json_body};
use crate::result::{local_error, local_ok, map_body, not_implemented};

const UP_TIMEOUT: Duration = Duration::from_secs(120);
const NOT_A_SPEC: &str = "sourceRef is not a rig spec";

pub(crate) async fn rig_start(ctx: &Ctx, args: RigStartArgs) -> CallToolResult {
    if let Some(feature) = refused_start(&args) {
        return not_implemented(feature);
    }
    let yaml = match read_spec(Path::new(&args.source_ref)).await {
        Ok(yaml) => yaml,
        Err(err) => return local_error(&json!({"error": err.to_string()})),
    };
    if crewmux_spec::parse(&yaml).is_err() {
        return local_error(&json!({"error": NOT_A_SPEC}));
    }
    one(
        ctx,
        Request {
            method: "POST",
            path: "/api/rigs/up".to_owned(),
            body: Some(yaml.into_bytes()),
            headers: Vec::new(),
            timeout: Some(UP_TIMEOUT),
        },
    )
    .await
}

pub(crate) async fn rig_stop(ctx: &Ctx, args: RigStopArgs) -> CallToolResult {
    if let Some(feature) = refused_stop(&args) {
        return not_implemented(feature);
    }
    let snapshot = if args.snapshot == Some(true) {
        match posted(ctx, "/api/snapshot/create", json!({"rig": &args.rig_id})).await {
            Posted::Ok(body) => Some(body),
            Posted::Failed(result) => return result,
        }
    } else {
        None
    };
    let down = match posted(ctx, "/api/rigs/down", json!({"rig": &args.rig_id})).await {
        Posted::Ok(body) => body,
        Posted::Failed(result) => return result,
    };
    match snapshot {
        Some(snapshot) => local_ok(&json!({"snapshot": snapshot, "down": down})),
        None => local_ok(&down),
    }
}

pub(crate) async fn rig_list(ctx: &Ctx, _args: NoArgs) -> CallToolResult {
    one(
        ctx,
        Request {
            method: "GET",
            path: "/api/rigs/summary".to_owned(),
            body: None,
            headers: Vec::new(),
            timeout: None,
        },
    )
    .await
}

fn refused_start(args: &RigStartArgs) -> Option<&'static str> {
    if args.plan == Some(true) {
        Some("plan")
    } else if args.auto_approve == Some(true) {
        Some("autoApprove")
    } else if args.target_root.is_some() {
        Some("targetRoot")
    } else {
        None
    }
}

fn refused_stop(args: &RigStopArgs) -> Option<&'static str> {
    if args.delete == Some(true) {
        Some("delete")
    } else if args.force == Some(true) {
        Some("force")
    } else {
        None
    }
}

enum Posted {
    Ok(Value),
    Failed(CallToolResult),
}

async fn posted(ctx: &Ctx, path: &str, body: Value) -> Posted {
    let Ok(bytes) = json_body(&body) else {
        return Posted::Failed(local_error(&json!({"error": "could not encode request"})));
    };
    match ctx
        .exchange(Request {
            method: "POST",
            path: path.to_owned(),
            body: Some(bytes),
            headers: Vec::new(),
            timeout: None,
        })
        .await
    {
        Ok(response) => posted_body(response.status, &response.body),
        Err(err) if Ctx::is_down(&err) => Posted::Failed(crate::result::daemon_unavailable()),
        Err(err) if Ctx::payload_rejected(&err) => {
            Posted::Failed(local_error(&json!({"error": "payload_rejected"})))
        }
        Err(err) => Posted::Failed(local_error(&json!({"error": err.to_string()}))),
    }
}

fn posted_body(status: u16, body: &str) -> Posted {
    let failed =
        status >= 400 || crate::result::map_body(status, body, |_| false).is_error == Some(true);
    if failed {
        Posted::Failed(map_body(status, body, |_| false))
    } else {
        let value = serde_json::from_str(body).unwrap_or_else(|_| Value::String(body.to_owned()));
        Posted::Ok(value)
    }
}

async fn read_spec(path: &Path) -> Result<String, Error> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || std::fs::read_to_string(path))
        .await
        .map_err(|err| Error::Message(err.to_string()))?
        .map_err(Error::from)
}

pub(super) async fn one(ctx: &Ctx, request: Request) -> CallToolResult {
    match ctx.exchange(request).await {
        Ok(response) => map_body(response.status, &response.body, |_| false),
        Err(err) if Ctx::is_down(&err) => crate::result::daemon_unavailable(),
        Err(err) if Ctx::payload_rejected(&err) => {
            local_error(&json!({"error": "payload_rejected"}))
        }
        Err(err) => local_error(&json!({"error": err.to_string()})),
    }
}
