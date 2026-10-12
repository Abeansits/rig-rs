//! Cursor-verified answers for known provider dialogs.
//!
//! Enter is sent only after a fresh capture shows the cursor on the requested
//! choice. Bypass consent and the MCP dialog have no automatic choice.

mod drive;

pub use drive::{Answered, DialogDriver, DialogOption, Escalate, MuxDialogDriver, auto_choice};
