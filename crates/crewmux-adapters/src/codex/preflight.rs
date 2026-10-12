//! `codex login status` before every launch and resume.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

/// Bound for `codex login status` when the caller does not pass a budget.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// What `codex login status` reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preflight {
    /// Exit 0.
    LoggedIn,
    /// Exit 1 and the output contains "Not logged in".
    NotLoggedIn,
    /// Exit 1 and the output says the Codex home does not exist.
    CodexHomeMissing,
    /// A missing binary, a timeout, or any other exit.
    ///
    /// Launch does not create a session.
    Unrecognised {
        /// Process exit code, or `-1` when the status had none.
        code: i32,
        /// Combined stdout and stderr, or the spawn/timeout reason.
        out: String,
    },
}

/// Run `codex login status` with `HOME` and `CODEX_HOME` set.
pub async fn codex_preflight(home: &Path, codex_home: &Path) -> Preflight {
    codex_preflight_bin(Path::new("codex"), home, codex_home, DEFAULT_TIMEOUT).await
}

/// Same as [`codex_preflight`], using `bin` instead of `codex` on `PATH`.
///
/// `timeout` bounds the child. A hang, a missing binary, and any other
/// failure stay in [`Preflight::Unrecognised`] so launch can refuse the session.
pub async fn codex_preflight_bin(
    bin: &Path,
    home: &Path,
    codex_home: &Path,
    timeout: Duration,
) -> Preflight {
    let run = tokio::process::Command::new(bin)
        .arg("login")
        .arg("status")
        .env("HOME", home)
        .env("CODEX_HOME", codex_home)
        .stdin(Stdio::null())
        .output();
    match tokio::time::timeout(timeout, run).await {
        Ok(Ok(output)) => {
            let mut out = String::from_utf8_lossy(&output.stdout).into_owned();
            out.push_str(&String::from_utf8_lossy(&output.stderr));
            let code = output.status.code().unwrap_or(-1);
            classify_preflight(code, &out)
        }
        Ok(Err(err)) => Preflight::Unrecognised {
            code: -1,
            out: format!("codex login status failed: {err}"),
        },
        Err(_elapsed) => Preflight::Unrecognised {
            code: -1,
            out: "codex login status timed out".to_owned(),
        },
    }
}

/// Classify a captured exit code and combined output.
#[must_use]
pub fn classify_preflight(code: i32, out: &str) -> Preflight {
    if code == 0 {
        return Preflight::LoggedIn;
    }
    let lower = out.to_ascii_lowercase();
    if code == 1 && lower.contains("not logged in") {
        return Preflight::NotLoggedIn;
    }
    let missing_home = lower.contains("codex home")
        && (lower.contains("does not exist") || lower.contains("error finding"));
    if code == 1 && missing_home {
        return Preflight::CodexHomeMissing;
    }
    Preflight::Unrecognised {
        code,
        out: out.to_owned(),
    }
}
