// Derived from openrig packages/cli/src/mcp-server.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! `rig_start`, `rig_stop`, and `rig_list`.

use std::path::Path;
use std::time::Duration;

use rmcp::model::CallToolResult;
use serde_json::json;

use crate::args::{NoArgs, RigStartArgs, RigStopArgs};
use crate::error::Error;
use crate::exchange::{Ctx, Request};
use crate::result::{daemon_unavailable, local_error, map_body};

const UP_TIMEOUT: Duration = Duration::from_secs(120);

pub(crate) async fn rig_start(ctx: &Ctx, args: RigStartArgs) -> CallToolResult {
    let _ = (args.plan, args.auto_approve, args.target_root.as_deref());
    let yaml = match read_spec(Path::new(&args.source_ref)).await {
        Ok(yaml) => yaml,
        Err(err) => return local_error(&json!({"error": err.to_string()})),
    };
    one(
        ctx,
        Request {
            method: "POST",
            path: "/api/rigs/up".to_owned(),
            body: Some(yaml.into_bytes()),
            headers: Vec::new(),
            timeout: Some(UP_TIMEOUT),
        },
        false,
    )
    .await
}

pub(crate) async fn rig_stop(ctx: &Ctx, args: RigStopArgs) -> CallToolResult {
    let _ = (args.delete, args.force, args.snapshot);
    let body = json!({"rig": args.rig_id});
    let Ok(bytes) = crate::exchange::json_body(&body) else {
        return local_error(&json!({"error": "could not encode rig_stop"}));
    };
    one(
        ctx,
        Request {
            method: "POST",
            path: "/api/rigs/down".to_owned(),
            body: Some(bytes),
            headers: Vec::new(),
            timeout: None,
        },
        false,
    )
    .await
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
        false,
    )
    .await
}

async fn read_spec(path: &Path) -> Result<String, Error> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || std::fs::read_to_string(path))
        .await
        .map_err(|err| Error::Message(err.to_string()))?
        .map_err(Error::from)
}

pub(super) async fn one(ctx: &Ctx, request: Request, structural: bool) -> CallToolResult {
    match ctx.exchange(request).await {
        Ok(response) => map_body(response.status, &response.body, |data| {
            structural && crate::result::unlaunched(data)
        }),
        Err(err) if Ctx::is_down(&err) => daemon_unavailable(),
        Err(err) => local_error(&json!({"error": err.to_string()})),
    }
}
