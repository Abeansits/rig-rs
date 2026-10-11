//! Child processes for doctor probes. `Command::output` is disallowed.

use std::path::Path;
use std::process::{Command, Stdio};

/// Exit code and stdout of one finished child.
pub(crate) struct Captured {
    /// `status.code()`, or 1 when the status has none.
    pub code: i32,
    /// Standard output, lossy-decoded.
    pub stdout: String,
}

/// Spawn `program` and wait. Returns `None` when the spawn itself fails.
#[must_use]
pub(crate) fn capture(program: &Path, args: &[String]) -> Option<Captured> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command.spawn().ok()?;
    let output = child.wait_with_output().ok()?;
    Some(Captured {
        code: output.status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
    })
}

/// First `name` on `path_env` that is a file.
#[must_use]
pub(crate) fn which(path_env: &str, name: &str) -> Option<std::path::PathBuf> {
    for dir in path_env.split(':') {
        if dir.is_empty() {
            continue;
        }
        let candidate = Path::new(dir).join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}
