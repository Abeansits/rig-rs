//! Repository tasks: the differential harness, the parity ledger, and source lints.
//!
//! `cargo xtask diff` compares TypeScript `OpenRig` v0.6.9 with the `rig` binary.
//! `cargo xtask ledger` seeds and checks `parity/ledger.csv`.
//! `cargo xtask lint-mux` rejects a human-client signal built from attachment.
//! `cargo xtask lint` checks provider config writers.

mod error;
mod lint;
mod lint_mux;
mod output;

pub mod diff;
pub mod ledger;
pub mod record_tui;

pub use error::{Error, io_err};
pub use lint::{lint_claude_json, lint_window_size};
pub use lint_mux::{lint_mux_at, workspace_root as lint_workspace_root};
pub use output::{err, out};
