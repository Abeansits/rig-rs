// Derived from openrig packages/daemon/src/adapters/tmux-exec.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Argv-only tmux execution. A shell binary is rejected before anything is spawned.

use std::ffi::OsStr;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use tokio::process::Command;

use crate::argv::invocation;
use crate::error::MuxError;

/// Successful stdout from one tmux command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecOutput {
    /// Standard output, lossy-decoded as UTF-8.
    pub stdout: String,
    /// Standard error, lossy-decoded as UTF-8.
    pub stderr: String,
}

/// Runs tmux argument vectors. Implementations must not invoke a shell.
#[async_trait]
pub trait TmuxExec: Send + Sync {
    /// Run `args` as tmux arguments (without the binary or `-L`).
    async fn run(&self, args: &[String]) -> Result<ExecOutput, MuxError>;
}

/// `tokio::process` exec against one tmux binary and socket.
#[derive(Clone, Debug)]
pub(crate) struct ProcessExec {
    binary: PathBuf,
    socket: String,
}

impl ProcessExec {
    /// `binary` is usually `tmux`. `socket` is the `-L` name (`rig-rs` by default).
    #[must_use]
    pub(crate) fn new(binary: PathBuf, socket: impl Into<String>) -> Self {
        Self {
            binary,
            socket: socket.into(),
        }
    }
}

#[async_trait]
impl TmuxExec for ProcessExec {
    async fn run(&self, args: &[String]) -> Result<ExecOutput, MuxError> {
        ensure_argv_only(self.binary.as_os_str())?;
        let tmux_bin = self.binary.display().to_string();
        let full = invocation(&tmux_bin, &self.socket, args);
        run_argv(&full).await
    }
}

/// Refuse to execute a shell. The pane-died hook is a tmux `run-shell` string, not a `Command`.
pub(crate) fn ensure_argv_only(program: &OsStr) -> Result<(), MuxError> {
    let name = program.to_string_lossy();
    let owned = PathBuf::from(name.as_ref());
    let file_name = owned
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let file_name = file_name.as_str();
    if matches!(file_name, "sh" | "bash" | "dash" | "zsh" | "ksh") {
        return Err(MuxError::Control(format!(
            "shell execution is not allowed (refusing `{file_name}`)"
        )));
    }
    Ok(())
}

pub(crate) async fn run_argv(argv: &[String]) -> Result<ExecOutput, MuxError> {
    let (tmux_bin, rest) = argv
        .split_first()
        .ok_or_else(|| MuxError::Control("empty tmux argv".to_owned()))?;
    ensure_argv_only(OsStr::new(tmux_bin))?;
    let mut command = Command::new(tmux_bin);
    command.args(rest);
    command.env_remove("TMUX");
    command.env_remove("TMUX_TMPDIR");
    command.env("TERM", "xterm-256color");
    command.kill_on_drop(true);
    let output = match tokio::time::timeout(Duration::from_secs(15), command.output()).await {
        Ok(result) => result?,
        Err(_) => {
            return Err(MuxError::Exec {
                argv: argv.to_vec(),
                code: None,
                stderr: "tmux command timed out".to_owned(),
            });
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if output.status.success() {
        return Ok(ExecOutput { stdout, stderr });
    }
    let mut detail = stderr;
    if detail.is_empty() {
        detail = stdout;
    }
    if is_not_found(&detail) {
        return Err(MuxError::NotFound(detail));
    }
    Err(MuxError::Exec {
        argv: argv.to_vec(),
        code: output.status.code(),
        stderr: detail,
    })
}

pub(crate) fn is_not_found(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("can't find session")
        || lower.contains("session not found")
        || lower.contains("can't find pane")
        || lower.contains("no such pane")
        || lower.contains("can't find window")
}

pub(crate) fn is_absence(stderr: &str) -> bool {
    is_not_found(stderr)
        || stderr.contains("no server running")
        || stderr.contains("error connecting to")
        || stderr.contains("no current target")
}

/// Spawn the long-lived control client: `tmux -L sock -C new-session -A -s crewmux-control`.
pub(crate) fn control_command(binary: &PathBuf, socket: &str) -> Command {
    let mut command = Command::new(binary);
    command
        .arg("-L")
        .arg(socket)
        .arg("-f")
        .arg("/dev/null")
        .arg("-C");
    command.args([
        "new-session",
        "-A",
        "-s",
        crate::types::CONTROL_SESSION,
        "-x",
        "80",
        "-y",
        "24",
    ]);
    command.env_remove("TMUX");
    command.env_remove("TMUX_TMPDIR");
    command.env("TERM", "xterm-256color");
    command.kill_on_drop(true);
    command.stdin(std::process::Stdio::piped());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::null());
    command
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{ProcessExec, TmuxExec, ensure_argv_only};
    use crate::error::MuxError;

    #[test]
    fn rejects_a_shell_without_spawning() {
        let err = ensure_argv_only(std::ffi::OsStr::new("sh")).expect_err("sh");
        assert!(err.to_string().contains("shell"), "{err}");
        for name in ["bash", "/bin/dash", "zsh"] {
            assert!(
                ensure_argv_only(std::ffi::OsStr::new(name)).is_err(),
                "{name}"
            );
        }
    }

    #[tokio::test]
    async fn process_exec_refuses_shell_binary() {
        let exec = ProcessExec::new(PathBuf::from("sh"), "rig-rs");
        let err = exec
            .run(&["-c".to_owned(), "echo hi".to_owned()])
            .await
            .expect_err("run");
        assert!(matches!(err, MuxError::Control(_)), "{err}");
    }
}
