//! Test fixture binary. Not part of a `--no-default-features` build.

use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    match crewmux_adapters::run_stub_agent() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            drop(write_err(&err.to_string()));
            ExitCode::from(1)
        }
    }
}

fn write_err(text: &str) -> std::io::Result<()> {
    writeln!(std::io::stderr(), "{text}")
}
