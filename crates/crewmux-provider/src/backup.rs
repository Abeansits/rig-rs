//! First-touch backups under `$RIG_HOME/backups/<file-id>/first-touch.orig`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::atomic::AtomicWrite;
use crate::error::ProviderError;

/// Backup root. The first capture of a file is kept; later captures do not overwrite it.
#[derive(Debug, Clone)]
pub struct Backups {
    root: PathBuf,
}

impl Backups {
    /// Backups live at `$RIG_HOME/backups`.
    #[must_use]
    pub fn new(rig_home: &Path) -> Self {
        Self {
            root: rig_home.join("backups"),
        }
    }

    /// Copy `path` once, if it exists and no backup has been stored for it yet.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Io`] when the backup directory or file cannot be written.
    pub fn capture_once(&self, path: &Path) -> Result<Option<PathBuf>, ProviderError> {
        if !path.is_file() {
            return Ok(None);
        }
        let dir = self.root.join(file_id(path));
        if has_backup(&dir)? {
            return Ok(None);
        }
        fs::create_dir_all(&dir)?;
        set_mode(&dir, 0o700)?;
        let dest = dir.join(backup_name());
        let bytes = fs::read(path)?;
        AtomicWrite::write(&dest, &bytes, Some(0o600))?;
        Ok(Some(dest))
    }
}

fn backup_name() -> &'static str {
    "first-touch.orig"
}

fn set_mode(path: &Path, bits: u32) -> Result<(), ProviderError> {
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(bits);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

fn has_backup(dir: &Path) -> Result<bool, ProviderError> {
    if !dir.exists() {
        return Ok(false);
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.path().extension().and_then(|ext| ext.to_str()) == Some("orig") {
            return Ok(true);
        }
    }
    Ok(false)
}

fn file_id(path: &Path) -> String {
    let text = path.to_string_lossy();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    // Hex only, so the backup directory name cannot carry `..` or a separator.
    format!("{hash:016x}")
}

use std::os::unix::fs::PermissionsExt;
