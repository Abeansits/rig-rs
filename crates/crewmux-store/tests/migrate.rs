//! Schema v1: apply once, refuse a newer file.
#![expect(
    clippy::expect_used,
    reason = "file-mode helpers abort the integration test on setup failure"
)]

use std::os::unix::fs::{MetadataExt, PermissionsExt};

use crewmux_store::{MIGRATIONS, Store};

#[test]
fn empty_database_applies_migrations_and_reopen_is_a_noop() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("rig.db");
    let expected = i64::try_from(MIGRATIONS.len()).expect("len");
    let store = Store::open(&path).expect("open");
    assert_eq!(store.schema_version().expect("version"), expected);
    drop(store);
    let again = Store::open(&path).expect("reopen");
    assert_eq!(again.schema_version().expect("version"), expected);
}

#[test]
fn newer_schema_is_refused() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("rig.db");
    drop(Store::open(&path).expect("open"));
    let conn = rusqlite::Connection::open(&path).expect("sqlite");
    conn.pragma_update(None, "user_version", 99i64)
        .expect("bump");
    drop(conn);
    let message = match Store::open(&path) {
        Ok(_store) => panic!("newer schema should be refused"),
        Err(err) => err.to_string(),
    };
    assert!(
        message.contains("newer") && message.contains("99"),
        "{message}"
    );
}

#[test]
fn database_files_reopen_at_owner_read_write() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("rig.db");
    drop(Store::open(&path).expect("open"));
    widen(&path);
    let wal = sibling(&path, "-wal");
    let shm = sibling(&path, "-shm");
    if wal.exists() {
        widen(&wal);
    }
    if shm.exists() {
        widen(&shm);
    }
    let store = Store::open(&path).expect("reopen");
    assert!(wal.exists(), "wal");
    assert!(shm.exists(), "shm");
    assert_eq!(mode_bits(&path), 0o600);
    assert_eq!(mode_bits(&wal), 0o600);
    assert_eq!(mode_bits(&shm), 0o600);
    drop(store);
}

fn widen(path: &std::path::Path) {
    let perms = std::fs::Permissions::from_mode(0o644);
    std::fs::set_permissions(path, perms).expect("widen");
}

fn sibling(path: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    let mut owned = path.as_os_str().to_os_string();
    owned.push(suffix);
    std::path::PathBuf::from(owned)
}

fn mode_bits(path: &std::path::Path) -> u32 {
    std::fs::metadata(path).expect("meta").mode() & 0o777
}
