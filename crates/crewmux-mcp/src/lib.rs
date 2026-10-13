//! MCP stdio server backed by the daemon API client.
//!
//! `rig mcp` speaks MCP on stdin and stdout. Tools call the daemon through
//! `crewmux-api`. Spec validation is local. `agent_add` does not call the daemon.

mod args;
mod caller;
mod error;
mod exchange;
mod names;
mod ops;
mod result;
mod schema;
mod server;
mod stdio_frame;

pub use caller::{Caller, caller_from_pair};
pub use error::Error;
pub use names::{LEGACY_NAMES, RIG_MCP_NAME, TOOL_NAMES, is_rig_tool};
pub use schema::{SchemaMode, compat};
pub use server::{RigMcp, serve_stdio, serve_stdio_blocking};
pub use stdio_frame::serve_io;
