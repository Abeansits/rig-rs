// Derived from openrig packages/cli/src/atomic-text-write.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
//! Replace a file by writing a temp file in the same directory, fsyncing, then renaming.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::ProviderError;

/// Publishes bytes with a same-directory temp file, fsync, and rename.
#[derive(Debug)]
pub struct AtomicWrite;

static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);

impl AtomicWrite {
    /// Replace `path` with `bytes`.
    ///
    /// When `mode` is `Some`, the published file uses those permission bits.
    /// When it is `None`, an existing file keeps its mode and a new file is `0o600`.
    /// A symlink to a regular file is followed: the link stays, and the target is renamed.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Io`] when the temp file cannot be created or renamed.
    /// The destination is not opened for writing until the rename, so a failure before
    /// that leaves the previous bytes in place.
    pub fn write(path: &Path, bytes: &[u8], mode: Option<u32>) -> Result<(), ProviderError> {
        let staged = stage(path, bytes, mode)?;
        publish(staged)
    }
}

/// A temp file that has been fsynced but not yet renamed over the destination.
#[derive(Debug)]
pub struct StagedWrite {
    temp: PathBuf,
    destination: PathBuf,
    armed: bool,
}

impl StagedWrite {
    /// Rename the temp file over the destination.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Io`] when the rename fails. The destination is unchanged
    /// and the temp file is removed.
    pub fn rename(mut self) -> Result<(), ProviderError> {
        sync_parent(&self.destination)?;
        fs::rename(&self.temp, &self.destination)?;
        self.armed = false;
        sync_parent(&self.destination)?;
        Ok(())
    }

    /// Path of the staged temp file, used by the crash-before-rename test.
    #[must_use]
    pub fn temp_path(&self) -> &Path {
        &self.temp
    }
}

impl Drop for StagedWrite {
    fn drop(&mut self) {
        if self.armed && self.temp.is_file() {
            drop(fs::remove_file(&self.temp));
        }
    }
}

/// Write and fsync a temp file next to `path` without renaming it.
///
/// Dropping the result deletes the temp file and leaves `path` untouched.
/// That is the crash window between the staged write and the rename.
///
/// # Errors
///
/// Returns [`ProviderError::Io`] when the temp file cannot be created.
pub fn stage(path: &Path, bytes: &[u8], mode: Option<u32>) -> Result<StagedWrite, ProviderError> {
    let destination = follow_file_symlink(path)?;
    if let Some(parent) = destination.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    if let Ok(meta) = fs::symlink_metadata(&destination)
        && !meta.file_type().is_file()
        && !meta.file_type().is_symlink()
    {
        write_in_place(&destination, bytes)?;
        return Ok(StagedWrite {
            temp: destination.clone(),
            destination,
            armed: false,
        });
    }
    let temp = temp_path(&destination);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    let bits = mode.unwrap_or_else(|| existing_mode(&destination).unwrap_or(0o600));
    set_mode(&temp, bits)?;
    Ok(StagedWrite {
        temp,
        destination,
        armed: true,
    })
}

fn publish(staged: StagedWrite) -> Result<(), ProviderError> {
    if staged.armed {
        staged.rename()
    } else {
        Ok(())
    }
}

fn write_in_place(path: &Path, bytes: &[u8]) -> Result<(), ProviderError> {
    let mut file = OpenOptions::new().write(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn temp_path(destination: &Path) -> PathBuf {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let name = format!(".rig-rs.tmp-{}-{seq}", std::process::id());
    parent.join(name)
}

fn existing_mode(path: &Path) -> Option<u32> {
    let meta = fs::metadata(path).ok()?;
    Some(meta.permissions().mode() & 0o777)
}

fn set_mode(path: &Path, bits: u32) -> Result<(), ProviderError> {
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(bits & 0o777);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

fn sync_parent(path: &Path) -> Result<(), ProviderError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    let dir = File::open(parent)?;
    dir.sync_all()?;
    Ok(())
}

fn follow_file_symlink(path: &Path) -> Result<PathBuf, ProviderError> {
    let mut target = path.to_path_buf();
    for _ in 0..40 {
        let meta = match fs::symlink_metadata(&target) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(target),
            Err(error) => return Err(error.into()),
        };
        if !meta.file_type().is_symlink() {
            return Ok(target);
        }
        let link = fs::read_link(&target)?;
        let next = if link.is_absolute() {
            link
        } else {
            target.parent().unwrap_or_else(|| Path::new(".")).join(link)
        };
        match fs::symlink_metadata(&next) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "refusing to write through a dangling symlink",
                )
                .into());
            }
            Err(error) => return Err(error.into()),
            Ok(_) => target = next,
        }
    }
    Err(ProviderError::Parse {
        file: path.to_path_buf(),
        msg: "too many symlinks".to_owned(),
    })
}

trait ModeOpen {
    fn mode(&mut self, bits: u32) -> &mut Self;
}

impl ModeOpen for OpenOptions {
    fn mode(&mut self, bits: u32) -> &mut Self {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptionsExt::mode(self, bits)
    }
}

use std::os::unix::fs::PermissionsExt;
