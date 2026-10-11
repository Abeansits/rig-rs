//! Errors from the multiplexer.

use std::io;

use crewmux_types::Version;

/// Require `found` to be at least [`Version::minimum`].
pub(crate) fn ensure_supported(found: &Version) -> Result<(), MuxError> {
    if found.is_supported() {
        Ok(())
    } else {
        Err(MuxError::TmuxTooOld {
            found: found.clone(),
            min: Version::minimum(),
        })
    }
}

/// Failure from a multiplexer operation.
#[derive(Debug, thiserror::Error)]
pub enum MuxError {
    /// The named session or pane does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// The caller asked for something tmux must not do.
    #[error("{0}")]
    Forbidden(&'static str),
    /// The installed tmux is older than the supported minimum.
    #[error("tmux {found} is older than minimum {min}")]
    TmuxTooOld {
        /// Version reported by `tmux -V`.
        found: Version,
        /// Oldest version this crate will drive.
        min: Version,
    },
    /// A tmux process exited unsuccessfully.
    #[error("command {argv:?} exited {code:?}: {stderr}")]
    Exec {
        /// Argv that was run, including the tmux binary.
        argv: Vec<String>,
        /// Process exit code, when the OS reported one.
        code: Option<i32>,
        /// Combined stderr (and stdout, when useful).
        stderr: String,
    },
    /// Control-mode protocol or caller-input failure.
    #[error("control mode: {0}")]
    Control(String),
    /// An OS I/O failure outside a tmux exit status.
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[cfg(test)]
mod tests {
    use crewmux_types::parse_version;

    use super::ensure_supported;

    #[test]
    fn supported_releases_pass_the_gate() {
        for raw in ["tmux 3.3\n", "tmux 3.5a", "tmux next-3.6"] {
            let version = parse_version(raw).expect("parse");
            ensure_supported(&version).expect("gate");
        }
    }

    #[test]
    fn older_than_3_3_names_both_versions() {
        let version = parse_version("tmux 3.2a").expect("parse");
        let err = ensure_supported(&version).expect_err("too old");
        let message = err.to_string();
        assert!(message.contains("3.2a"), "{message}");
        assert!(message.contains("3.3"), "{message}");
    }
}
