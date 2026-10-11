//! Per-seat `OpenCode` config. It is never written under the user's `~/.config/opencode` directory.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::atomic::AtomicWrite;
use crate::error::ProviderError;

/// Permission block honoured by `opencode` when `OPENCODE_CONFIG` points at the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenCodePermission {
    /// Edit permission (`allow`, `ask`, or `deny`).
    pub edit: String,
    /// Shell permission.
    pub bash: String,
    /// Web fetch permission.
    pub webfetch: String,
}

/// JSON document written next to a seat, not into `~/.config/opencode`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenCodeSeatConfig {
    /// Permission block.
    pub permission: OpenCodePermission,
}

impl OpenCodeSeatConfig {
    /// Write `opencode.json` inside `seat_dir`.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Parse`] when `seat_dir` is under `~/.config/opencode`,
    /// and [`ProviderError::Io`] when the file cannot be written.
    pub fn write(&self, seat_dir: &Path) -> Result<PathBuf, ProviderError> {
        if is_user_opencode_dir(seat_dir) {
            return Err(ProviderError::Parse {
                file: seat_dir.to_path_buf(),
                msg: "refusing to write the user OpenCode config".to_owned(),
            });
        }
        let path = seat_dir.join("opencode.json");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = serde_json::to_string_pretty(self).map_err(|err| ProviderError::Parse {
            file: path.clone(),
            msg: err.to_string(),
        })?;
        AtomicWrite::write(&path, body.as_bytes(), Some(0o600))?;
        Ok(path)
    }
}

fn is_user_opencode_dir(path: &Path) -> bool {
    let mut parts = path.components();
    let mut prev_config = false;
    for part in parts.by_ref() {
        let text = part.as_os_str().to_string_lossy();
        if prev_config && text == "opencode" {
            return true;
        }
        prev_config = text == ".config";
    }
    false
}
