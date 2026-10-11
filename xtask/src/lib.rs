//! Repository tasks: the differential harness and the parity ledger.
//!
//! `cargo xtask diff` compares TypeScript `OpenRig` v0.6.9 with the `rig` binary.
//! `cargo xtask ledger` seeds and checks `parity/ledger.csv`.

mod error;
mod lint;
mod output;

pub mod diff;
pub mod ledger;
pub mod record_tui;

pub use lint::{lint_claude_json, lint_window_size};

pub use error::{Error, io_err};
pub use output::{err, out};
