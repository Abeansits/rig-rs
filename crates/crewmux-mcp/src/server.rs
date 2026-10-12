// Derived from openrig packages/cli/src/mcp-server.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! rmcp server. Unknown tools are a protocol error. Tool failures are `isError`.

use std::sync::Arc;

use crewmux_api::Client;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, ErrorCode, Implementation, ListToolsResult,
    ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt, tool, tool_router};
use serde_json::Value;

use crate::args::{
    AgentAddArgs, AgentSendArgs, ChatReadArgs, ChatSendArgs, NoArgs, ReadScreenArgs, RigStartArgs,
    RigStopArgs, SnapshotRestoreArgs, SnapshotRigArgs, YamlArgs,
};
use crate::caller::Caller;
use crate::error::Error;
use crate::exchange::Ctx;
use crate::names::{RIG_MCP_NAME, is_rig_tool};
use crate::ops;
use crate::schema::{SchemaMode, compat};

/// Stdio MCP server. Tools call the daemon through [`Client`].
#[derive(Clone)]
pub struct RigMcp {
    tool_router: rmcp::handler::server::router::tool::ToolRouter<Self>,
    api: Client,
    caller: Caller,
    schema: SchemaMode,
}

impl RigMcp {
    /// Build a server for an already-chosen socket, caller, and schema mode.
    #[must_use]
    pub fn new(api: Client, caller: Caller, schema: SchemaMode) -> Self {
        Self {
            tool_router: Self::tool_router(),
            api,
            caller,
            schema,
        }
    }

    /// Socket from `RIG_HOME`, caller from the seat variables, schema from `RIG_MCP_SCHEMA_COMPAT`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::PartialSeatEnv`] or [`Error::Seat`] when the seat pair is incomplete or invalid.
    pub fn from_env() -> Result<Self, Error> {
        let caller = Caller::from_env()?;
        let home = crewmux_api::rig_home();
        let api = Client::over_uds(crewmux_api::daemon_socket(&home));
        Ok(Self::new(api, caller, SchemaMode::from_env()))
    }

    fn ctx(&self) -> Ctx {
        Ctx {
            api: self.api.clone(),
            caller: self.caller.clone(),
        }
    }

    fn listed_tools(&self) -> Vec<rmcp::model::Tool> {
        let mut tools = self.tool_router.list_all();
        if self.schema == SchemaMode::Compat {
            for tool in &mut tools {
                apply_compat(tool);
            }
        }
        tools
    }
}

fn apply_compat(tool: &mut rmcp::model::Tool) {
    let value = Value::Object(tool.input_schema.as_ref().clone());
    if let Value::Object(map) = compat(value) {
        tool.input_schema = Arc::new(map);
    }
}

#[tool_router]
impl RigMcp {
    /// Bootstrap a rig by posting the spec file at `sourceRef` to `POST /api/rigs/up`.
    #[tool(
        name = "rig_start",
        description = "Bootstrap a rig by posting the YAML file at sourceRef to the daemon. plan, autoApprove, and targetRoot are accepted and not applied."
    )]
    async fn rig_start(
        &self,
        Parameters(args): Parameters<RigStartArgs>,
    ) -> rmcp::model::CallToolResult {
        ops::rig_start(&self.ctx(), args).await
    }

    /// Stop one rig via `POST /api/rigs/down`.
    #[tool(
        name = "rig_stop",
        description = "Tear down one rig. The daemon body is {\"rig\": rigId}. delete, force, and snapshot are accepted and not applied."
    )]
    async fn rig_stop(
        &self,
        Parameters(args): Parameters<RigStopArgs>,
    ) -> rmcp::model::CallToolResult {
        ops::rig_stop(&self.ctx(), args).await
    }

    /// List rigs via `GET /api/rigs/summary`.
    #[tool(
        name = "rig_list",
        description = "List rigs from GET /api/rigs/summary."
    )]
    async fn rig_list(&self, Parameters(args): Parameters<NoArgs>) -> rmcp::model::CallToolResult {
        ops::rig_list(&self.ctx(), args).await
    }

    /// List seats via `GET /api/seats`.
    #[tool(name = "agent_list", description = "List seats from GET /api/seats.")]
    async fn agent_list(
        &self,
        Parameters(args): Parameters<NoArgs>,
    ) -> rmcp::model::CallToolResult {
        ops::agent_list(&self.ctx(), args).await
    }

    /// Post a member to the pod-members route.
    #[tool(
        name = "agent_add",
        description = "Add a member by posting to /api/rigs/{rigId}/pods/{podNamespace}/members. The current daemon has no such route; a 404 is a tool error."
    )]
    async fn agent_add(
        &self,
        Parameters(args): Parameters<AgentAddArgs>,
    ) -> rmcp::model::CallToolResult {
        ops::agent_add(&self.ctx(), args).await
    }

    /// Deliver text through `POST /api/send`.
    #[tool(
        name = "agent_send",
        description = "Deliver text to a seat through POST /api/send, which runs lineage and pre-send checks. force does not bypass those checks. force combined with waitForIdleSeconds is rejected before any request. waitForIdleSeconds does not extend the timeout."
    )]
    async fn agent_send(
        &self,
        Parameters(args): Parameters<AgentSendArgs>,
    ) -> rmcp::model::CallToolResult {
        ops::agent_send(&self.ctx(), args).await
    }

    /// Capture a seat screen.
    #[tool(
        name = "agent_read_screen",
        description = "Read a seat's live screen through POST /api/seats/capture."
    )]
    async fn agent_read_screen(
        &self,
        Parameters(args): Parameters<ReadScreenArgs>,
    ) -> rmcp::model::CallToolResult {
        ops::agent_read_screen(&self.ctx(), args).await
    }

    /// Send a chat message after resolving the rig name.
    #[tool(
        name = "chat_send",
        description = "Resolve rigName via GET /api/rigs/summary, then POST /api/rigs/{id}/chat/send. Two daemon calls. Seat headers are forwarded when this process is a seat."
    )]
    async fn chat_send(
        &self,
        Parameters(args): Parameters<ChatSendArgs>,
    ) -> rmcp::model::CallToolResult {
        ops::chat_send(&self.ctx(), args).await
    }

    /// Read recent chat history after resolving the rig name.
    #[tool(
        name = "chat_read",
        description = "Resolve rigName via GET /api/rigs/summary, then GET /api/rigs/{id}/chat/history?limit=20. Two daemon calls. This is history, not a stream."
    )]
    async fn chat_read(
        &self,
        Parameters(args): Parameters<ChatReadArgs>,
    ) -> rmcp::model::CallToolResult {
        ops::chat_read(&self.ctx(), args).await
    }

    /// Create a snapshot.
    #[tool(
        name = "snapshot_create",
        description = "Create a snapshot via POST /api/snapshot/create with {\"rig\": rigId}."
    )]
    async fn snapshot_create(
        &self,
        Parameters(args): Parameters<SnapshotRigArgs>,
    ) -> rmcp::model::CallToolResult {
        ops::snapshot_create(&self.ctx(), args).await
    }

    /// List snapshots.
    #[tool(
        name = "snapshot_list",
        description = "List snapshots via POST /api/snapshot/list with {\"rig\": rigId}."
    )]
    async fn snapshot_list(
        &self,
        Parameters(args): Parameters<SnapshotRigArgs>,
    ) -> rmcp::model::CallToolResult {
        ops::snapshot_list(&self.ctx(), args).await
    }

    /// Restore a snapshot.
    #[tool(
        name = "snapshot_restore",
        description = "Restore via POST /api/restore with {\"rig\": rigId, \"snapshot\": snapshotId}."
    )]
    async fn snapshot_restore(
        &self,
        Parameters(args): Parameters<SnapshotRestoreArgs>,
    ) -> rmcp::model::CallToolResult {
        ops::snapshot_restore(&self.ctx(), args).await
    }

    /// Validate a rig spec locally.
    #[tool(
        name = "spec_validate_rig",
        description = "Validate RigSpec YAML locally with crewmux-spec. No daemon call. Result is {valid, errors}."
    )]
    async fn spec_validate_rig(
        &self,
        Parameters(args): Parameters<YamlArgs>,
    ) -> rmcp::model::CallToolResult {
        ops::spec_validate_rig(args)
    }

    /// Check that agent YAML is a mapping with a name.
    #[tool(
        name = "spec_validate_agent",
        description = "Check that agent YAML is a mapping with a non-empty name. This is not a full AgentSpec schema. No daemon call. Result is {valid, errors}."
    )]
    async fn spec_validate_agent(
        &self,
        Parameters(args): Parameters<YamlArgs>,
    ) -> rmcp::model::CallToolResult {
        ops::spec_validate_agent(args)
    }

    /// Report daemon health.
    #[tool(
        name = "daemon_health",
        description = "GET /healthz. When the socket is down, returns {\"running\": false} and does not start the daemon."
    )]
    async fn daemon_health(
        &self,
        Parameters(args): Parameters<NoArgs>,
    ) -> rmcp::model::CallToolResult {
        ops::daemon_health(&self.ctx(), args).await
    }
}

impl ServerHandler for RigMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(RIG_MCP_NAME, env!("CARGO_PKG_VERSION")))
    }

    fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        let tools = self.listed_tools();
        std::future::ready(Ok(ListToolsResult {
            tools,
            ..Default::default()
        }))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + Send + '_ {
        let known = is_rig_tool(request.name.as_ref());
        async move {
            if !known {
                return Err(ErrorData::new(
                    ErrorCode::METHOD_NOT_FOUND,
                    "unknown tool",
                    None,
                ));
            }
            let ctx = ToolCallContext::new(self, request, context);
            self.tool_router.call(ctx).await
        }
    }
}

/// Serve `server` on stdin and stdout until the client closes.
///
/// # Errors
///
/// Returns [`Error::Serve`] when the handshake or the session task fails.
pub async fn serve_stdio(server: RigMcp) -> Result<(), Error> {
    let running = server
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|err| Error::Serve(err.to_string()))?;
    running
        .waiting()
        .await
        .map_err(|err| Error::Serve(err.to_string()))?;
    Ok(())
}

/// Build a current-thread runtime and serve stdin and stdout.
///
/// The `rig` binary calls this so it does not start a runtime of its own.
///
/// # Errors
///
/// Returns [`Error`] from [`RigMcp::from_env`], the runtime, or [`serve_stdio`].
pub fn serve_stdio_blocking() -> Result<(), Error> {
    let server = RigMcp::from_env()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;
    runtime.block_on(serve_stdio(server))
}
