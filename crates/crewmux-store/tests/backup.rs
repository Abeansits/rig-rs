//! SQLite backup runs on a maintenance connection, not the writer thread.
#![expect(
    clippy::expect_used,
    reason = "setup failures abort the integration test"
)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use crewmux_store::{MIGRATIONS, Store};

#[test]
fn backup_refuses_an_in_memory_database() {
    let store = Store::open(std::path::Path::new(":memory:")).expect("open");
    let err = store
        .backup_maintenance(std::path::Path::new("/tmp/unused-slice8.db"), || {})
        .expect_err("memory");
    assert!(err.to_string().contains("writer"), "{err}");
}

fn seed_pages(store: &Store) {
    store
        .write(|conn| {
            conn.execute_batch("BEGIN")?;
            for index in 0..8_000 {
                conn.execute(
                    "INSERT INTO events (kind, payload, at_ms) VALUES ('note', ?1, ?2)",
                    rusqlite::params!["page-padding-xxxxxxxxxxxxxxxxxxxxxxxx", index],
                )?;
            }
            conn.execute_batch("COMMIT")?;
            Ok(())
        })
        .expect("seed");
}

#[test]
fn backup_does_not_stall_the_writer() {
    let dir = tempfile::tempdir().expect("temp");
    let path = dir.path().join("rig.db");
    let store = Arc::new(Store::open(&path).expect("open"));
    seed_pages(&store);
    let pages: i64 = store
        .read(|conn| Ok(conn.query_row("PRAGMA page_count", [], |row| row.get(0))?))
        .expect("pages");
    assert!(
        pages > 2,
        "backup step(1) needs more than one page, got {pages}"
    );

    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (wrote_tx, wrote_rx) = mpsc::channel();
    let writer = Arc::clone(&store);
    let handle = std::thread::spawn(move || {
        if entered_rx.recv_timeout(Duration::from_secs(3)).is_err() {
            wrote_tx
                .send(Err("between never ran".to_owned()))
                .expect("report");
            return;
        }
        let result = writer.write(|conn| {
            conn.execute(
                "INSERT INTO events (kind, payload, at_ms) VALUES ('ping', '1', 1)",
                [],
            )?;
            Ok(())
        });
        wrote_tx
            .send(result.map_err(|err| err.to_string()))
            .expect("report");
        release_tx.send(()).expect("release");
    });

    let dest = dir.path().join("snap.db");
    let once = AtomicBool::new(false);
    let mut wrote_during = false;
    store
        .backup_maintenance(&dest, || {
            if once
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                return;
            }
            entered_tx.send(()).expect("entered");
            wrote_during = release_rx.recv_timeout(Duration::from_secs(2)).is_ok();
        })
        .expect("backup");
    assert!(once.load(Ordering::SeqCst), "step(1) never yielded a page");
    assert!(
        wrote_during,
        "writer did not finish while the maintenance backup was between pages"
    );
    let wrote = wrote_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("writer");
    wrote.expect("insert");
    handle.join().expect("writer thread");

    let version = Store::file_user_version(&dest).expect("version");
    assert_eq!(version, i64::try_from(MIGRATIONS.len()).unwrap_or(0));
    let ping: i64 = store
        .read(|conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM events WHERE kind = 'ping'",
                [],
                |row| row.get(0),
            )?)
        })
        .expect("ping");
    assert_eq!(ping, 1);
}
