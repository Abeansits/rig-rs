//! Atomic replace: temp file, fsync, rename, mode, and first-touch backup.

use crewmux_provider::{AtomicWrite, Backups, stage};
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

#[test]
fn crash_before_rename() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    fs::write(&path, b"original").unwrap();
    let staged = stage(&path, b"next", None).unwrap();
    assert!(staged.temp_path().is_file());
    drop(staged);
    assert_eq!(fs::read(&path).unwrap(), b"original");
    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.to_string_lossy().contains(".rig-rs.tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn preserves_mode_and_backs_up_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secret.toml");
    fs::write(&path, b"first").unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(&path, permissions).unwrap();
    let inode = fs::metadata(&path).unwrap().ino();
    let backups = Backups::new(&dir.path().join("rig-home"));
    let backup = backups.capture_once(&path).unwrap().unwrap();
    AtomicWrite::write(&path, b"second", None).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"second");
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    assert_eq!(fs::read(&backup).unwrap(), b"first");
    assert!(backups.capture_once(&path).unwrap().is_none());
    assert_eq!(fs::read(&backup).unwrap(), b"first");
    assert_ne!(fs::metadata(&path).unwrap().ino(), inode);
}

#[test]
fn follows_a_symlink_to_a_regular_file() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.txt");
    let link = dir.path().join("link.txt");
    fs::write(&target, b"old").unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    AtomicWrite::write(&link, b"new", None).unwrap();
    assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(fs::read(&target).unwrap(), b"new");
}
