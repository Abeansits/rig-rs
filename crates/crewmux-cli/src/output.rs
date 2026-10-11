//! The only module in this crate that writes to the terminal.

/// Print `text` to stdout, adding a trailing newline when `text` has none.
#[expect(
    clippy::print_stdout,
    reason = "user-facing CLI output goes through this module"
)]
pub fn print_stdout(text: &str) {
    if text.is_empty() {
        return;
    }
    if text.ends_with('\n') {
        print!("{text}");
    } else {
        println!("{text}");
    }
}

/// Print `text` to stderr, adding a trailing newline when `text` has none.
#[expect(
    clippy::print_stderr,
    reason = "user-facing CLI errors go through this module"
)]
pub fn print_stderr(text: &str) {
    if text.is_empty() {
        return;
    }
    if text.ends_with('\n') {
        eprint!("{text}");
    } else {
        eprintln!("{text}");
    }
}

/// Write `rig: {message}` to stderr.
pub fn error_line(message: &str) {
    print_stderr(&format!("rig: {message}"));
}
