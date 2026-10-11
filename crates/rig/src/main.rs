//! The `rig` binary: dispatches to the CLI, daemon, TUI and MCP server.
//!
//! With no arguments, `--help`, or `--version`, this stays the slice 0 scaffold:
//! it prints nothing and exits 0. `rig tui` is the TUI front door.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("tui") {
        return ExitCode::SUCCESS;
    }
    match crewmux_cli::run_tui() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            crewmux_cli::report(&err);
            ExitCode::from(1)
        }
    }
}
