// Derived from openrig packages/cli/src/mcp-server.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Agent tools. `agent_send` posts to `POST /api/send`.
//!
//! That route delivers the text. A respawned stub in the same pane still receives
//! it, so this module does not claim a lineage check.

use rmcp::model::CallToolResult;
use serde_json::{Value, json};

use crate::args::{AgentAddArgs, AgentListArgs, AgentSendArgs, ReadScreenArgs};
use crate::exchange::{Ctx, Request, json_body};
use crate::result::{local_error, local_ok, not_implemented};

use super::rig::one;

pub(crate) async fn agent_list(ctx: &Ctx, args: AgentListArgs) -> CallToolResult {
    let summary = match get_json(ctx, "/api/rigs/summary").await {
        Ok(body) => body,
        Err(result) => return result,
    };
    let name = match rig_name(&summary, &args.rig_id) {
        Ok(name) => name,
        Err(result) => return result,
    };
    let seats = match get_json(ctx, "/api/seats").await {
        Ok(body) => body,
        Err(result) => return result,
    };
    local_ok(&seats_for(&seats, &name))
}

pub(crate) async fn agent_read_screen(ctx: &Ctx, args: ReadScreenArgs) -> CallToolResult {
    if let Some(feature) = refused_capture(&args) {
        return not_implemented(feature);
    }
    let Some(session) = args.session.filter(|text| !text.is_empty()) else {
        return local_error(&json!({"error": "session is required"}));
    };
    let mut body = json!({"seat": session});
    if let Some(lines) = args.lines
        && let Some(map) = body.as_object_mut()
    {
        map.insert("lines".to_owned(), json!(lines));
    }
    post_json(ctx, "/api/seats/capture".to_owned(), body).await
}

pub(crate) async fn agent_send(ctx: &Ctx, args: AgentSendArgs) -> CallToolResult {
    if args.force == Some(true) && args.wait_for_idle_seconds.is_some() {
        return local_error(&json!({
            "ok": false,
            "error": "waitForIdleSeconds cannot be combined with force"
        }));
    }
    if let Some(seconds) = args.wait_for_idle_seconds {
        if !seconds.is_finite() || seconds <= 0.0 {
            return local_error(&json!({"error": "waitForIdleSeconds must be positive"}));
        }
        return not_implemented("waitForIdleSeconds");
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
    )
    .await
}

pub(crate) async fn agent_add(_ctx: &Ctx, args: AgentAddArgs) -> CallToolResult {
    let AgentAddArgs {
        rig_id,
        pod_namespace,
        member,
        edges,
        rig_root,
    } = args;
    drop(rig_id);
    drop(pod_namespace);
    drop(member);
    drop(edges);
    drop(rig_root);
    not_implemented("agent_add")
}

fn refused_capture(args: &ReadScreenArgs) -> Option<&'static str> {
    if args.rig.is_some() {
        Some("rig")
    } else if args.pod.is_some() {
        Some("pod")
    } else {
        None
    }
}

async fn get_json(ctx: &Ctx, path: &str) -> Result<Value, CallToolResult> {
    let response = match ctx
        .exchange(Request {
            method: "GET",
            path: path.to_owned(),
            body: None,
            headers: Vec::new(),
            timeout: None,
        })
        .await
    {
        Ok(response) => response,
        Err(err) if Ctx::is_down(&err) => return Err(crate::result::daemon_unavailable()),
        Err(err) if Ctx::payload_rejected(&err) => {
            return Err(local_error(&json!({"error": "payload_rejected"})));
        }
        Err(err) => return Err(local_error(&json!({"error": err.to_string()}))),
    };
    if response.status >= 400 {
        return Err(crate::result::map_body(
            response.status,
            &response.body,
            |_| false,
        ));
    }
    serde_json::from_str(&response.body)
        .map_err(|_| local_error(&json!({"error": "daemon response was not JSON"})))
}

fn rig_name(summary: &Value, rig_id: &str) -> Result<String, CallToolResult> {
    let rows = summary.as_array().cloned().unwrap_or_default();
    let by_id: Vec<_> = rows
        .iter()
        .filter(|row| row.get("id").and_then(Value::as_str) == Some(rig_id))
        .collect();
    if let Some(row) = by_id.first().filter(|_| by_id.len() == 1) {
        let name = row.get("name").and_then(Value::as_str).unwrap_or(rig_id);
        return Ok(name.to_owned());
    }
    let by_name: Vec<_> = rows
        .iter()
        .filter(|row| row.get("name").and_then(Value::as_str) == Some(rig_id))
        .collect();
    match by_name.len() {
        1 => Ok(rig_id.to_owned()),
        0 => Err(local_error(
            &json!({"error": format!("Rig '{rig_id}' not found")}),
        )),
        count => Err(local_error(&json!({
            "error": format!("Rig '{rig_id}' is ambiguous — {count} rigs share that name")
        }))),
    }
}

fn seats_for(body: &Value, rig_name: &str) -> Value {
    let Some(rigs) = body.get("rigs").and_then(Value::as_array) else {
        return json!([]);
    };
    for rig in rigs {
        if rig.get("name").and_then(Value::as_str) == Some(rig_name) {
            return rig.get("seats").cloned().unwrap_or_else(|| json!([]));
        }
    }
    json!([])
}

async fn post_json(ctx: &Ctx, path: String, body: Value) -> CallToolResult {
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
    )
    .await
}
