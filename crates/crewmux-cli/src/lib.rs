//! Command tree and output for the `rig` CLI.
//!
//! `setup` plans provider, tmux, skill, and install steps. `doctor` is read-only.
//! The binary dispatches `tui` before clap so `rig --help` stays the setup/doctor list.

mod command;
mod doctor_cmd;
mod doctor_eval;
mod doctor_facts;
mod doctor_fixture;
mod doctor_live;
mod error;
mod herdr_probe;
mod output;
mod paths;
mod setup_apply;
mod setup_cmd;
mod setup_plan;
mod spawn;
mod tui_cmd;

pub use command::{Cli, Command};
pub use error::Error;
pub use output::{error_line, print_stderr, print_stdout};
pub use setup_plan::{SetupPlan, SetupStep};
pub use tui_cmd::{report, run as run_tui};

use clap::Parser;

use command::Command as RigCommand;
use paths::Paths;

/// Parse process arguments and run `setup` or `doctor`.
///
/// # Errors
///
/// Returns [`Error`] when `HOME` is unset or a command fails after argument parsing.
pub fn run() -> Result<i32, Error> {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            drop(error.print());
            return Ok(error.exit_code());
        }
    };
    let paths = Paths::capture()?;
    dispatch(cli.command, &paths)
}

fn dispatch(command: Option<RigCommand>, paths: &Paths) -> Result<i32, Error> {
    match command {
        Some(RigCommand::Setup(args)) => setup_cmd::run(paths, &args),
        Some(RigCommand::Doctor(args)) => doctor_cmd::run(paths, &args),
        None => {
            print_stdout("Usage: rig <setup|doctor> [--help]");
            Ok(0)
        }
    }
}
