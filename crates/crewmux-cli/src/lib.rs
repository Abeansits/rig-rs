//! Command tree, output formatting and daemon auto-start for the `rig` CLI.

mod output;
mod tui_cmd;

pub use output::error_line;
pub use tui_cmd::{report, run as run_tui};
