// Derived from openrig packages/cli/src/mcp-server.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Agent tools. `agent_send` goes through `POST /api/send`, which runs lineage checks.

use rmcp::model::CallToolResult;
use serde_json::{Value, json};

use crate::args::{AgentAddArgs, AgentSendArgs, NoArgs, ReadScreenArgs};
use crate::exchange::{Ctx, Request, encode_seg, json_body};
use crate::result::local_error;

use super::rig::one;

pub(crate) async fn agent_list(ctx: &Ctx, _args: NoArgs) -> CallToolResult {
    one(
        ctx,
        Request {
            method: "GET",
            path: "/api/seats".to_owned(),
            body: None,
            headers: Vec::new(),
            timeout: None,
        },
        false,
    )
    .await
}

pub(crate) async fn agent_read_screen(ctx: &Ctx, args: ReadScreenArgs) -> CallToolResult {
    let mut body = json!({"seat": args.session});
    if let Some(lines) = args.lines
        && let Some(map) = body.as_object_mut()
    {
        map.insert("lines".to_owned(), json!(lines));
    }
    post_json(ctx, "/api/seats/capture".to_owned(), body, false).await
}

pub(crate) async fn agent_send(ctx: &Ctx, args: AgentSendArgs) -> CallToolResult {
    if args.force == Some(true) && args.wait_for_idle_seconds.is_some() {
        return local_error(&json!({
            "ok": false,
            "error": "waitForIdleSeconds cannot be combined with force"
        }));
    }
    if let Some(seconds) = args.wait_for_idle_seconds
        && (!(seconds.is_finite()) || seconds <= 0.0)
    {
        return local_error(&json!({"error": "waitForIdleSeconds must be positive"}));
    }
    let body = json!({
        "to": args.session,
        "body": args.text,
        "verify": args.verify.unwrap_or(false),
        "from": ctx.caller.sender_label(),
    });
    let Ok(bytes) = json_body(&body) else {
        return local_error(&json!({"error": "could not encode agent_send"}));
    };
    one(
        ctx,
        Request {
            method: "POST",
            path: "/api/send".to_owned(),
            body: Some(bytes),
            headers: ctx.caller.headers(),
            timeout: None,
        },
        false,
    )
    .await
}

pub(crate) async fn agent_add(ctx: &Ctx, args: AgentAddArgs) -> CallToolResult {
    let mut body = json!({"member": args.member});
    if let Some(map) = body.as_object_mut() {
        if let Some(edges) = args.edges {
            map.insert("edges".to_owned(), json!(edges));
        }
        if let Some(root) = args.rig_root {
            map.insert("rigRoot".to_owned(), json!(root));
        }
    }
    let path = format!(
        "/api/rigs/{}/pods/{}/members",
        encode_seg(&args.rig_id),
        encode_seg(&args.pod_namespace)
    );
    post_json(ctx, path, body, true).await
}

async fn post_json(ctx: &Ctx, path: String, body: Value, structural: bool) -> CallToolResult {
    let Ok(bytes) = json_body(&body) else {
        return local_error(&json!({"error": "could not encode request"}));
    };
    one(
        ctx,
        Request {
            method: "POST",
            path,
            body: Some(bytes),
            headers: Vec::new(),
            timeout: None,
        },
        structural,
    )
    .await
}
