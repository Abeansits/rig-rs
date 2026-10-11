//! Launch integrations for external terminals.
//!
//! Herdr is attach-only: this crate talks to a running control socket and never
//! starts or stops the Herdr server. Ghostty is a thin `osascript` launcher.

mod error;
mod ghostty;
mod herdr;

pub use error::Error;
pub use ghostty::{open_window, script};
pub use herdr::open_pane;
