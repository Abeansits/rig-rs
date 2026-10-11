//! User-facing text. Every print in the CLI goes through here.

use serde_json::Value;

/// Print one stdout line.
#[expect(
    clippy::print_stdout,
    reason = "user-facing CLI output is confined to this module"
)]
pub fn stdout_line(text: &str) {
    println!("{text}");
}

/// Print a JSON document as one line.
pub fn stdout_json(value: &Value) {
    match serde_json::to_string(value) {
        Ok(text) => stdout_line(&text),
        Err(err) => stderr_line(&err.to_string()),
    }
}

/// Print one stderr line.
#[expect(
    clippy::print_stderr,
    reason = "user-facing CLI errors are confined to this module"
)]
pub fn stderr_line(text: &str) {
    eprintln!("{text}");
}

/// Human-readable seats when `--json` was not passed.
pub fn stdout_seats(value: &Value) {
    let seats = value
        .get("seats")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| {
            value
                .get("rigs")
                .and_then(Value::as_array)
                .and_then(|rigs| rigs.first().cloned())
                .and_then(|rig| rig.get("seats").cloned())
                .and_then(|seats| seats.as_array().cloned())
        });
    let Some(seats) = seats else {
        stdout_json(value);
        return;
    };
    for seat in seats {
        let name = field(&seat, "name");
        let runtime = field(&seat, "runtime");
        let state = field(&seat, "state");
        let session = field(&seat, "tmux_session");
        stdout_line(&format!("{name}\t{runtime}\t{state}\t{session}"));
    }
}

fn field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("-")
        .to_owned()
}

/// Print capture text, or the raw JSON when the text field is missing.
pub fn stdout_capture(value: &Value) {
    match value.get("text").and_then(Value::as_str) {
        Some(text) => stdout_line(text),
        None => stdout_json(value),
    }
}

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
