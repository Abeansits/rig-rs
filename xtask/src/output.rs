//! Operator-facing text for the `xtask` binary.

/// Print a report on stdout.
#[expect(
    clippy::print_stdout,
    reason = "xtask is the operator-facing repo tool"
)]
pub fn out(message: &str) {
    println!("{message}");
}

/// Print a failure on stderr.
#[expect(clippy::print_stderr, reason = "xtask reports failures on stderr")]
pub fn err(message: &str) {
    eprintln!("{message}");
}
