//! The `rig` binary: CLI verbs, and `rig daemon run` for the background server.

use std::process::ExitCode;

fn main() -> ExitCode {
    let code = match real_main() {
        Ok(code) => code,
        Err(err) => {
            crewmux_cli::output::stderr_line(&err.to_string());
            err.code()
        }
    };
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

fn real_main() -> Result<i32, crewmux_cli::Error> {
    let mut args = std::env::args();
    let _argv0 = args.next();
    if args.next().as_deref() == Some("daemon") {
        return Ok(daemon_command(args));
    }
    crewmux_cli::run()
}

fn daemon_command(mut args: impl Iterator<Item = String>) -> i32 {
    let command = args.next();
    if args.next().is_some() {
        crewmux_cli::output::stderr_line("rig daemon takes one subcommand");
        return 1;
    }
    match command.as_deref() {
        Some("run") => match crewmux_daemon::run() {
            Ok(()) => 0,
            Err(err) => {
                crewmux_cli::output::stderr_line(&err.to_string());
                1
            }
        },
        Some("status") => crewmux_cli::daemon_status(),
        Some(other) => {
            crewmux_cli::output::stderr_line(&format!(
                "unknown daemon command {other}; expected run or status"
            ));
            1
        }
        None => {
            crewmux_cli::output::stderr_line("usage: rig daemon <run|status>");
            1
        }
    }
}
