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

    /// User files tmux 3.2+ loads by default, after the system config.
    ///
    /// Order matches `TMUX_CONF`: `~/.tmux.conf`, then
    /// `$XDG_CONFIG_HOME/tmux/tmux.conf` when that variable is set, then
    /// `~/.config/tmux/tmux.conf`. A repeated path is listed once.
    #[must_use]
    pub(crate) fn user_tmux_configs(&self) -> Vec<PathBuf> {
        let mut paths = vec![self.tmux_conf.clone()];
        if let Some(xdg) = &self.xdg_config_home {
            push_unique(&mut paths, xdg.join("tmux").join("tmux.conf"));
        }
        push_unique(
            &mut paths,
            self.home.join(".config").join("tmux").join("tmux.conf"),
        );
        paths
    }

    /// Bytes of every file from [`Self::user_tmux_configs`] that can be read.
    /// Missing files are skipped. `source-file` directives are not followed.
    #[must_use]
    pub(crate) fn read_user_tmux_configs(&self) -> String {
        let mut out = String::new();
        for path in self.user_tmux_configs() {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            out.push_str(&text);
            if !text.ends_with('\n') {
                out.push('\n');
            }
        }
        out
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

fn push_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if paths.iter().all(|existing| existing != &path) {
        paths.push(path);
    }
}

/// Linux `sun_path` is 108 bytes including the trailing NUL.
pub(crate) const SUN_PATH_LIMIT: usize = 108;

/// The daemon socket path fits in [`SUN_PATH_LIMIT`].
#[must_use]
pub(crate) fn socket_path_fits(path: &Path) -> bool {
    path.as_os_str().len() < SUN_PATH_LIMIT
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;

    use crewmux_provider::global_window_size_manual;

    use super::Paths;

    fn vars(home: &std::path::Path, xdg: Option<&std::path::Path>) -> BTreeMap<String, String> {
        let mut vars = BTreeMap::new();
        vars.insert("HOME".to_owned(), home.display().to_string());
        if let Some(xdg) = xdg {
            vars.insert("XDG_CONFIG_HOME".to_owned(), xdg.display().to_string());
        }
        vars
    }

    #[test]
    fn xdg_tmux_conf_is_scanned_for_manual_window_size() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let xdg_default = home.join(".config").join("tmux");
        fs::create_dir_all(&xdg_default).unwrap();
        fs::write(xdg_default.join("tmux.conf"), "set -g window-size manual\n").unwrap();
        let paths = Paths::from_vars(&vars(&home, None)).unwrap();
        let text = paths.read_user_tmux_configs();
        assert!(global_window_size_manual(&text), "{text}");
        assert!(!home.join(".tmux.conf").exists());

        let other = dir.path().join("other-home");
        let xdg = dir.path().join("xdg");
        fs::create_dir_all(xdg.join("tmux")).unwrap();
        fs::write(
            xdg.join("tmux").join("tmux.conf"),
            "set -g window-size manual\n",
        )
        .unwrap();
        fs::create_dir_all(&other).unwrap();
        let custom = Paths::from_vars(&vars(&other, Some(&xdg))).unwrap();
        let custom_text = custom.read_user_tmux_configs();
        assert!(global_window_size_manual(&custom_text), "{custom_text}");
        assert!(!other.join(".tmux.conf").exists());
        assert!(
            !other
                .join(".config")
                .join("tmux")
                .join("tmux.conf")
                .exists()
        );
    }
}
