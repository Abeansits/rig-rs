//! Ghostty launcher. macOS only, via `osascript`. Other platforms return [`Error::Unsupported`].

use crate::error::Error;

/// Script that activates Ghostty. The command is carried as a comment the
/// operator can see.
#[must_use]
pub fn script(command: &str) -> String {
    format!("tell application \"Ghostty\"\n  activate\nend tell\n-- {command}\n")
}

/// Open a Ghostty window for `command`.
///
/// # Errors
///
/// Returns [`Error::Unsupported`] off macOS, or [`Error::Io`] when `osascript` fails.
pub async fn open_window(command: &[String]) -> Result<(), Error> {
    if !cfg!(target_os = "macos") {
        return Err(Error::Unsupported);
    }
    let joined = command.join(" ");
    let status = tokio::process::Command::new("osascript")
        .arg("-e")
        .arg(script(&joined))
        .status()
        .await
        .map_err(Error::Io)?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Protocol(format!("osascript exited {status}")))
    }
}

#[cfg(test)]
mod tests {
    use super::{open_window, script};

    #[test]
    fn script_names_ghostty() {
        let text = script("tmux attach");
        assert!(text.contains("tell application \"Ghostty\""));
        assert!(text.contains("tmux attach"));
    }

    #[tokio::test]
    async fn open_window_is_unsupported_off_macos() {
        if cfg!(target_os = "macos") {
            return;
        }
        let err = open_window(&["tmux".to_owned()])
            .await
            .expect_err("platform");
        assert!(matches!(err, crate::Error::Unsupported));
    }
}
