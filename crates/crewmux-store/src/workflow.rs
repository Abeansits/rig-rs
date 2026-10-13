//! The workflow document and the queue commit in one transaction.
//!
//! A crash hook returns before commit. A second connection opened while the
//! transaction is open still reads the previous rows.

use crewmux_messaging::QueueBook;
use rusqlite::{Connection, OptionalExtension, params};

use crate::error::Error;
use crate::queue::overwrite_book;

/// One seat the workflow inventory can see.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeatCoord {
    /// Rig name.
    pub rig: String,
    /// Seat name.
    pub name: String,
    /// Agent runtime.
    pub runtime: String,
}

/// Replace the queue and the single workflow document, then commit.
///
/// `crash` of `after-queue` or `before-commit` rolls the transaction back.
///
/// # Errors
///
/// Returns [`Error::CrashHook`] or a SQLite error. A crash leaves the previous
/// queue rows and the previous document in place.
pub fn commit_projection(
    conn: &mut Connection,
    book: &QueueBook,
    doc: &str,
    crash: Option<&str>,
) -> Result<(), Error> {
    let tx = conn.transaction()?;
    overwrite_book(&tx, book)?;
    if crash == Some("after-queue") {
        return Err(Error::CrashHook("after-queue"));
    }
    tx.execute(
        "INSERT INTO workflow_documents (id, doc) VALUES (1, ?1)
         ON CONFLICT(id) DO UPDATE SET doc = excluded.doc",
        params![doc],
    )?;
    if crash == Some("before-commit") {
        return Err(Error::CrashHook("before-commit"));
    }
    tx.commit()?;
    Ok(())
}

/// The workflow document, when one has been committed.
///
/// # Errors
///
/// Returns a SQLite error.
pub fn load_workflow_document(conn: &Connection) -> Result<Option<String>, Error> {
    let doc = conn
        .query_row(
            "SELECT doc FROM workflow_documents WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(doc)
}

/// Rig and seat names for owner resolution.
///
/// # Errors
///
/// Returns a SQLite error.
pub fn list_seat_coords(conn: &Connection) -> Result<Vec<SeatCoord>, Error> {
    let mut stmt = conn.prepare(
        "SELECT rigs.name, seats.name, seats.runtime
         FROM seats JOIN rigs ON seats.rig_id = rigs.id
         ORDER BY rigs.name, seats.name",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(SeatCoord {
            rig: row.get(0)?,
            name: row.get(1)?,
            runtime: row.get(2)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crewmux_core::Timestamp;
    use crewmux_messaging::{NewItem, QueueBook, QueueState, SessionRef};
    use crewmux_types::QItemId;

    use super::{commit_projection, load_workflow_document};
    use crate::queue::{load_book, overwrite_book};
    use crate::store::Store;

    fn book_with(id: &str) -> QueueBook {
        let mut book = QueueBook::new();
        let qid = QItemId::new(id).expect("id");
        book.create(
            NewItem {
                id: Some(qid),
                source: SessionRef::new("alice@rig").expect("session"),
                destination: SessionRef::new("bob@rig").expect("session"),
                body: id.to_owned(),
                priority: 0,
                tier: None,
                tags: Vec::new(),
                handed_off_from: None,
                last_nudge: None,
                explicit_human: false,
                fingerprint: String::new(),
            },
            Timestamp::from_millis(1_000),
        )
        .expect("create");
        book
    }

    #[test]
    fn crash_before_commit_keeps_the_previous_document() {
        let store = Store::open(PathBuf::from(":memory:").as_path()).expect("open");
        store
            .write(|conn| {
                let before = book_with("qitem-before");
                commit_projection(conn, &before, "before", None).expect("seed");
                let after = book_with("qitem-after");
                let crashed = commit_projection(conn, &after, "after", Some("after-queue"))
                    .expect_err("hook");
                assert_eq!(crashed.to_string(), "crash hook after-queue");
                assert_eq!(
                    load_workflow_document(conn).expect("doc").as_deref(),
                    Some("before")
                );
                assert_eq!(load_book(conn).expect("book").items().len(), 1);
                let crashed = commit_projection(conn, &after, "after", Some("before-commit"))
                    .expect_err("commit");
                assert_eq!(crashed.to_string(), "crash hook before-commit");
                assert_eq!(
                    load_book(conn).expect("still").items()[0].state,
                    QueueState::Pending
                );
                assert!(
                    load_book(conn).expect("still").items()[0]
                        .id
                        .as_str()
                        .contains("before")
                );
                commit_projection(conn, &after, "after", None).expect("ok");
                assert_eq!(
                    load_workflow_document(conn).expect("doc").as_deref(),
                    Some("after")
                );
                Ok(())
            })
            .expect("write");
    }

    #[test]
    fn reader_during_projection_sees_the_before_state() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let path = std::env::temp_dir().join(format!("rig-wf-{}-{nanos}.db", std::process::id()));
        drop(std::fs::remove_file(&path));
        let store = Store::open(&path).expect("open");
        let db_path = path.clone();
        let outcome = store.write(move |conn| {
            let before = book_with("qitem-live");
            commit_projection(conn, &before, "{\"n\":1}", None).expect("seed");
            let tx = conn.transaction()?;
            overwrite_book(&tx, &book_with("qitem-next"))?;
            tx.execute(
                "INSERT INTO workflow_documents (id, doc) VALUES (1, ?1)
                 ON CONFLICT(id) DO UPDATE SET doc = excluded.doc",
                rusqlite::params!["{\"n\":2}"],
            )?;
            let reader = rusqlite::Connection::open(&db_path)?;
            let seen = load_book(&reader).expect("reader book");
            assert_eq!(seen.items().len(), 1);
            assert!(seen.items()[0].id.as_str().contains("live"));
            assert_eq!(
                load_workflow_document(&reader)
                    .expect("reader doc")
                    .as_deref(),
                Some("{\"n\":1}")
            );
            tx.commit()?;
            let after = rusqlite::Connection::open(&db_path)?;
            assert!(
                load_book(&after).expect("after").items()[0]
                    .id
                    .as_str()
                    .contains("next")
            );
            assert_eq!(
                load_workflow_document(&after)
                    .expect("after doc")
                    .as_deref(),
                Some("{\"n\":2}")
            );
            Ok(())
        });
        drop(std::fs::remove_file(&path));
        outcome.expect("write");
    }
}
