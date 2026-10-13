// Derived from openrig packages/cli/src/mcp-server.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Chat tools resolve a rig name, then call the chat route.
//!
//! That is two daemon calls. Every other tool is one call, and spec validation is none.

use rmcp::model::CallToolResult;
use serde_json::json;

use crate::args::{ChatReadArgs, ChatSendArgs};
use crate::exchange::{Ctx, Request, encode_seg, json_body};
use crate::result::{daemon_unavailable, local_error, map_body};

use super::rig::one;

pub(crate) async fn chat_send(ctx: &Ctx, args: ChatSendArgs) -> CallToolResult {
    let crate::args::ChatSendArgs {
        rig_name,
        body,
        sender,
    } = args;
    drop(sender);
    let rig_id = match lookup(ctx, &rig_name).await {
        Lookup::One(id) => id,
        Lookup::Failed(result) => return result,
    };
    let Ok(bytes) = json_body(&json!({"body": body})) else {
        return local_error(&json!({"error": "could not encode chat_send"}));
    };
    one(
        ctx,
        Request {
            method: "POST",
            path: format!("/api/rigs/{}/chat/send", encode_seg(&rig_id)),
            body: Some(bytes),
            headers: ctx.caller.headers(),
            timeout: None,
        },
    )
    .await
}

pub(crate) async fn chat_read(ctx: &Ctx, args: ChatReadArgs) -> CallToolResult {
    let rig_id = match lookup(ctx, &args.rig_name).await {
        Lookup::One(id) => id,
        Lookup::Failed(result) => return result,
    };
    one(
        ctx,
        Request {
            method: "GET",
            path: format!("/api/rigs/{}/chat/history?limit=20", encode_seg(&rig_id)),
            body: None,
            headers: Vec::new(),
            timeout: None,
        },
    )
    .await
}

enum Lookup {
    One(String),
    Failed(CallToolResult),
}

async fn lookup(ctx: &Ctx, name: &str) -> Lookup {
    let rows = match summary(ctx).await {
        Ok(rows) => rows,
        Err(result) => return Lookup::Failed(result),
    };
    let matches: Vec<_> = rows.iter().filter(|row| row.name == name).collect();
    match matches.len() {
        1 => {
            if let Some(row) = matches.first() {
                Lookup::One(row.id.clone())
            } else {
                Lookup::Failed(name_error(name, 0))
            }
        }
        count => Lookup::Failed(name_error(name, count)),
    }
}

fn name_error(name: &str, count: usize) -> CallToolResult {
    let error = if count == 0 {
        format!("Rig '{name}' not found")
    } else {
        format!("Rig '{name}' is ambiguous — {count} rigs share that name")
    };
    local_error(&json!({"error": error}))
}

struct Row {
    id: String,
    name: String,
}

async fn summary(ctx: &Ctx) -> Result<Vec<Row>, CallToolResult> {
    let response = match ctx
        .exchange(Request {
            method: "GET",
            path: "/api/rigs/summary".to_owned(),
            body: None,
            headers: Vec::new(),
            timeout: None,
        })
        .await
    {
        Ok(response) => response,
        Err(err) if Ctx::is_down(&err) => return Err(daemon_unavailable()),
        Err(err) => return Err(local_error(&json!({"error": err.to_string()}))),
    };
    if response.status >= 400 {
        return Err(map_body(response.status, &response.body, |_| false));
    }
    let rows: Vec<serde_json::Value> = serde_json::from_str(&response.body).unwrap_or_default();
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some(Row {
                id: row.get("id")?.as_str()?.to_owned(),
                name: row.get("name")?.as_str()?.to_owned(),
            })
        })
        .collect())
}
