//! The `rig` binary: dispatches to the CLI, daemon, TUI and MCP server.
//!
//! `rig tui` is intercepted before clap so `rig --help` stays `doctor`, `help`,
//! and `setup`.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("tui") {
        match crewmux_cli::run_tui() {
            Ok(()) => {}
            Err(err) => {
                crewmux_cli::report(&err);
                exit_process(1);
            }
        }
        return;
    }
    if let Err(error) = run() {
        crewmux_cli::print_stderr(&format!("{error:#}"));
        exit_process(1);
    }
}

fn run() -> anyhow::Result<()> {
    let code = crewmux_cli::run()?;
    if code != 0 {
        exit_process(code);
    }
    Ok(())
}

#[expect(
    clippy::exit,
    reason = "non-zero status is the doctor and usage contract"
)]
fn exit_process(code: i32) -> ! {
    std::process::exit(code);
}
