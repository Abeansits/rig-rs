//! Repository tasks: the differential harness, the parity ledger, and mux lints.
//!
//! `cargo xtask diff` compares TypeScript `OpenRig` v0.6.9 with the `rig` binary.
//! `cargo xtask ledger` seeds and checks `parity/ledger.csv`.
//! `cargo xtask lint-mux` rejects a human-client signal built from attachment.

mod error;
mod lint_mux;
mod output;

pub mod diff;
pub mod ledger;

pub use error::{Error, io_err};
pub use lint_mux::{lint_mux_at, workspace_root as lint_workspace_root};
pub use output::{err, out};
