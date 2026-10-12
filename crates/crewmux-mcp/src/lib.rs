//! MCP stdio server backed by the daemon API client.
//!
//! `rig mcp` speaks MCP on stdin and stdout. Every state-changing tool is one
//! daemon HTTP call. Spec validation is local and does not open the socket.

mod args;
mod caller;
mod error;
mod exchange;
mod names;
mod ops;
mod result;
mod schema;
mod server;

pub use caller::{Caller, caller_from_pair};
pub use error::Error;
pub use names::{LEGACY_NAMES, RIG_MCP_NAME, TOOL_NAMES, is_rig_tool};
pub use schema::{SchemaMode, compat};
pub use server::{RigMcp, serve_stdio, serve_stdio_blocking};
