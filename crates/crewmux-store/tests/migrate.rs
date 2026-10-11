//! Schema v1: apply once, refuse a newer file.

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
