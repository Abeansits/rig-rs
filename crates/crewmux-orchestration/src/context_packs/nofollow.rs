//! Pack reads that do not follow symlinks.
//!
//! Each component is checked with `symlink_metadata`, then the final file is
//! opened with `O_NOFOLLOW` so a swap between the check and the read cannot
//! return the target's bytes.

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use std::os::unix::fs::OpenOptionsExt;

/// Linux `O_NOFOLLOW` is `0400000`. macOS is `0x0100`.
#[cfg(any(target_os = "linux", target_os = "android"))]
const O_NOFOLLOW: i32 = 0o400_000;
#[cfg(any(target_os = "macos", target_os = "ios"))]
const O_NOFOLLOW: i32 = 0x0100;

/// The first symlink under `root` along `rel`, if any.
#[must_use]
pub(crate) fn symlink_under(root: &Path, rel: &Path) -> Option<PathBuf> {
    let mut cursor = root.to_path_buf();
    for component in rel.components() {
        match component {
            Component::Normal(segment) => cursor.push(segment),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Some(cursor);
            }
        }
        if is_symlink(&cursor) {
            return Some(cursor);
        }
    }
    None
}

/// Read `path` without following a final symlink.
///
/// # Errors
///
/// Returns the open or read error. A symlink final component fails the open
/// (`O_NOFOLLOW`).
pub(crate) fn read_regular(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW)
        .open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// UTF-8 text from [`read_regular`].
///
/// # Errors
///
/// Returns the open error, or [`std::io::ErrorKind::InvalidData`] when the
/// bytes are not UTF-8.
pub(crate) fn read_regular_text(path: &Path) -> std::io::Result<String> {
    let bytes = read_regular(path)?;
    String::from_utf8(bytes)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))
}

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}
