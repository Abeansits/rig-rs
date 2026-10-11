// Derived from openrig packages/cli/src/herdr-launch-config.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
//! Private Herdr launch config. An existing personal config is left untouched.
//! This module never starts or stops a Herdr server.

use std::fs;
use std::path::{Path, PathBuf};

use crate::atomic::AtomicWrite;
use crate::error::ProviderError;

/// Environment and socket used to choose the launch config path.
#[derive(Debug, Clone)]
pub struct HerdrLaunchConfig<'a> {
    /// Process environment. `HOME` and `XDG_CONFIG_HOME` select the personal file.
    pub env_home: &'a Path,
    /// `XDG_CONFIG_HOME`, when set.
    pub xdg_config_home: Option<&'a Path>,
    /// `HERDR_CONFIG_PATH`, when the caller set it.
    pub config_path: Option<&'a Path>,
    /// Herdr socket the private file is keyed by.
    pub socket_path: &'a str,
    /// Terminal columns. Sidebars collapse below 160.
    pub columns: Option<u32>,
}

impl HerdrLaunchConfig<'_> {
    /// Return the personal config when it exists. Otherwise write a private file
    /// beside that path and return the private path.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Io`] when the private file cannot be written, and
    /// [`ProviderError::Parse`] when the private path would alias the personal path.
    pub fn prepare(&self) -> Result<PathBuf, ProviderError> {
        let source = self.personal_path();
        if source.is_file() {
            return Ok(source);
        }
        let collapsed = self.columns.is_none_or(|columns| columns < 160);
        let bytes = format!("onboarding = false\n\n[ui]\nsidebar_start_collapsed = {collapsed}\n");
        let key = private_key(&source, self.socket_path, collapsed);
        let target = source
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(format!(".rig-rs-herdr-{key}.toml"));
        if target == source {
            return Err(ProviderError::Parse {
                file: source,
                msg: "the private Herdr config must differ from the source config".to_owned(),
            });
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
            drop(fs::set_permissions(parent, mode_bits(0o700)));
        }
        if target.is_symlink() {
            drop(fs::remove_file(&target));
        }
        AtomicWrite::write(&target, bytes.as_bytes(), Some(0o600))?;
        Ok(target)
    }

    fn personal_path(&self) -> PathBuf {
        if let Some(path) = self.config_path {
            return path.to_path_buf();
        }
        let root = match self.xdg_config_home {
            Some(path) => path.to_path_buf(),
            None => self.env_home.join(".config"),
        };
        root.join("herdr").join("config.toml")
    }
}

fn private_key(source: &Path, socket: &str, collapsed: bool) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in source.to_string_lossy().as_bytes() {
        hash = mix(hash, *byte);
    }
    hash = mix(hash, 0);
    for byte in socket.as_bytes() {
        hash = mix(hash, *byte);
    }
    hash = mix(hash, u8::from(collapsed));
    format!("{hash:016x}")
}

fn mix(hash: u64, byte: u8) -> u64 {
    hash.wrapping_mul(0x0000_0100_0000_01b3) ^ u64::from(byte)
}

fn mode_bits(bits: u32) -> std::fs::Permissions {
    use std::os::unix::fs::PermissionsExt;
    std::fs::Permissions::from_mode(bits)
}
