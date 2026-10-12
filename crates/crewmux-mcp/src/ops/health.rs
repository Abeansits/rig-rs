//! `daemon_health`. A missing socket is `{"running": false}`, not a tool error.

use rmcp::model::CallToolResult;

use crate::args::NoArgs;
use crate::exchange::{Ctx, Request};
use crate::result::{local_ok, map_body};

pub(crate) async fn daemon_health(ctx: &Ctx, _args: NoArgs) -> CallToolResult {
    match ctx
        .exchange(Request {
            method: "GET",
            path: "/healthz".to_owned(),
            body: None,
            headers: Vec::new(),
            timeout: None,
        })
        .await
    {
        Ok(response) => map_body(response.status, &response.body, |_| false),
        Err(err) if Ctx::is_down(&err) => local_ok(&serde_json::json!({"running": false})),
        Err(err) => map_body(
            503,
            &serde_json::json!({"error": err.to_string()}).to_string(),
            |_| false,
        ),
    }
}
