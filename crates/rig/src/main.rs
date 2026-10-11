//! The `rig` binary: dispatches to the CLI.

fn main() {
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
