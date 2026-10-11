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
        return match crewmux_daemon::run() {
            Ok(()) => Ok(0),
            Err(err) => {
                crewmux_cli::output::stderr_line(&err.to_string());
                Ok(1)
            }
        };
    }
    crewmux_cli::run()
}
