//! Per-seat Claude files and the guarded trust pre-accept.
//!
//! [`trust`] is the only module allowed to open the Claude user config for writing.

mod seat;
mod trust;

pub use seat::{ClaudeSeatFiles, ClaudeSeatSettings, McpServerEntry};
pub use trust::{ClaudeTrust, FileStamp, ProcessProbe, TrustOutcome, user_config_basename};
