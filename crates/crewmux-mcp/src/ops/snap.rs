// Derived from openrig packages/cli/src/mcp-server.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Snapshot tools call the slice 8 routes. The body is `{"rig": ...}`, not a path segment.

use rmcp::model::CallToolResult;
use serde_json::json;

use crate::args::{SnapshotRestoreArgs, SnapshotRigArgs};
use crate::exchange::{Ctx, json_body};
use crate::result::local_error;

use super::rig::one;

pub(crate) async fn snapshot_create(ctx: &Ctx, args: SnapshotRigArgs) -> CallToolResult {
    post(ctx, "/api/snapshot/create", json!({"rig": args.rig_id})).await
}

pub(crate) async fn snapshot_list(ctx: &Ctx, args: SnapshotRigArgs) -> CallToolResult {
    post(ctx, "/api/snapshot/list", json!({"rig": args.rig_id})).await
}

pub(crate) async fn snapshot_restore(ctx: &Ctx, args: SnapshotRestoreArgs) -> CallToolResult {
    post(
        ctx,
        "/api/restore",
        json!({"rig": args.rig_id, "snapshot": args.snapshot_id}),
    )
    .await
}

async fn post(ctx: &Ctx, path: &str, body: serde_json::Value) -> CallToolResult {
    let Ok(bytes) = json_body(&body) else {
        return local_error(&json!({"error": "could not encode snapshot request"}));
    };
    one(
        ctx,
        crate::exchange::Request {
            method: "POST",
            path: path.to_owned(),
            body: Some(bytes),
            headers: Vec::new(),
            timeout: None,
        },
        false,
    )
    .await
}
