//! Private directories and files for `RIG_HOME`.
//!
//! Ancestors are created with the process umask and never chmod'd. Only a
//! directory this call creates is set to 0700. Files opened here are 0600
//! even when they already existed.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

/// Create `dir` at mode 0700 when it is missing.
///
/// Returns `true` when this call created `dir`. An existing path is left
/// unchanged, including its mode. Parents outside `dir` are created when
/// needed and never chmod'd.
///
/// # Errors
///
/// Returns an error when a parent cannot be created or the mode cannot be set.
pub fn create_private(dir: &Path) -> io::Result<bool> {
    if let Some(parent) = dir.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::create_dir(dir) {
        Ok(()) => {
            set_mode(dir, 0o700)?;
            Ok(true)
        }
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(err) => Err(err),
    }
}

/// Open `path` and force mode 0600.
///
/// The create mode is 0600, so a umask cannot loosen a new file. An existing
/// file is tightened with `set_permissions` after the open. `options` carries
/// the read, write, create, and append flags.
///
/// # Errors
///
/// Returns an error when the file cannot be opened or the mode cannot be set.
pub fn open_private(path: &Path, options: &mut OpenOptions) -> io::Result<File> {
    let file = options.mode(0o600).open(path)?;
    let mut perms = file.metadata()?.permissions();
    perms.set_mode(0o600);
    file.set_permissions(perms)?;
    Ok(file)
}

fn set_mode(dir: &Path, mode: u32) -> io::Result<()> {
    let mut perms = std::fs::metadata(dir)?.permissions();
    perms.set_mode(mode);
    std::fs::set_permissions(dir, perms)
}
