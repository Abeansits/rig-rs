//! `$HOME`, `$RIG_HOME`, and the provider files setup and doctor read.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::Error;

/// Paths and environment captured once at startup. Nothing here calls `set_var`.
#[derive(Debug, Clone)]
pub(crate) struct Paths {
    /// `$HOME`.
    pub home: PathBuf,
    /// `$RIG_HOME`, or `$HOME/.rig-rs` when that variable is unset.
    pub rig_home: PathBuf,
    /// Set when the process exported `RIG_HOME`.
    pub rig_home_explicit: bool,
    /// Codex `config.toml`.
    pub codex_config: PathBuf,
    /// User tmux config.
    pub tmux_conf: PathBuf,
    /// `$XDG_CONFIG_HOME`, when set.
    pub xdg_config_home: Option<PathBuf>,
    /// `$HERDR_CONFIG_PATH`, when set.
    pub herdr_config: Option<PathBuf>,
    /// `$PATH`.
    pub path_env: String,
    /// `$COLUMNS`, when it parses.
    pub columns: Option<u32>,
    /// Process environment, used to see whether a Codex `env_key` is set.
    pub vars: BTreeMap<String, String>,
}

impl Paths {
    /// Read the process environment.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Usage`] when `HOME` is unset, so the CLI never falls
    /// back to a real home directory.
    pub(crate) fn capture() -> Result<Self, Error> {
        let vars: BTreeMap<String, String> = std::env::vars().collect();
        Self::from_vars(&vars)
    }

    /// Build paths from an explicit map. Tests pass a temp `HOME`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Usage`] when `HOME` is missing.
    pub(crate) fn from_vars(vars: &BTreeMap<String, String>) -> Result<Self, Error> {
        let home = vars
            .get("HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| Error::Usage("HOME is not set".to_owned()))?;
        let (rig_home, rig_home_explicit) = match vars.get("RIG_HOME") {
            Some(value) if !value.is_empty() => (PathBuf::from(value), true),
            _ => (home.join(".rig-rs"), false),
        };
        let codex_home = match vars.get("CODEX_HOME") {
            Some(value) if !value.is_empty() => PathBuf::from(value),
            _ => home.join(".codex"),
        };
        Ok(Self {
            tmux_conf: home.join(".tmux.conf"),
            xdg_config_home: vars
                .get("XDG_CONFIG_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
            herdr_config: vars
                .get("HERDR_CONFIG_PATH")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
            path_env: vars.get("PATH").cloned().unwrap_or_default(),
            columns: vars.get("COLUMNS").and_then(|text| text.parse().ok()),
            vars: vars.clone(),
            home,
            rig_home,
            rig_home_explicit,
            codex_config: codex_home.join("config.toml"),
        })
    }

    /// `$RIG_HOME/seats/default`.
    #[must_use]
    pub(crate) fn seat_dir(&self) -> PathBuf {
        self.rig_home.join("seats").join("default")
    }

    /// `$RIG_HOME/skills`.
    #[must_use]
    pub(crate) fn skills_dir(&self) -> PathBuf {
        self.rig_home.join("skills")
    }

    /// True when `name` is a non-empty environment variable.
    #[must_use]
    pub(crate) fn var_set(&self, name: &str) -> bool {
        self.vars
            .get(name)
            .is_some_and(|value| !value.trim().is_empty())
    }
}

/// Linux `sun_path` is 108 bytes including the trailing NUL.
pub(crate) const SUN_PATH_LIMIT: usize = 108;

/// The daemon socket path fits in [`SUN_PATH_LIMIT`].
#[must_use]
pub(crate) fn socket_path_fits(path: &Path) -> bool {
    path.as_os_str().len() < SUN_PATH_LIMIT
}
