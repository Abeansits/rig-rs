//! Queue rows, the transition log, and delivery receipts.
//!
//! A handoff is one transaction. A crash hook returns before commit, and the
//! transaction rolls back, so the file stays on the before state.

use crewmux_core::Timestamp;
use crewmux_messaging::{QueueBook, QueueItem, Transition};
use rusqlite::{Connection, params};

use crate::error::Error;

/// Replace the live queue tables with `book`.
///
/// `crash` of `after-items` or `before-commit` errors before commit.
///
/// # Errors
///
/// Returns [`Error::CrashHook`] or a SQLite or JSON error. A crash leaves the
/// previous rows in place.
pub fn replace_book(
    conn: &mut Connection,
    book: &QueueBook,
    crash: Option<&str>,
) -> Result<(), Error> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM queue_items", [])?;
    for item in book.items() {
        insert_item(&tx, &item)?;
    }
    if crash == Some("after-items") {
        return Err(Error::CrashHook("after-items"));
    }
    tx.execute("DELETE FROM queue_transitions", [])?;
    for row in book.all_transitions() {
        insert_transition(&tx, &row)?;
    }
    if crash == Some("before-commit") {
        return Err(Error::CrashHook("before-commit"));
    }
    tx.commit()?;
    Ok(())
}

/// Replace the queue tables without starting a transaction.
///
/// # Errors
///
/// Returns a SQLite or JSON error.
pub(crate) fn overwrite_book(conn: &Connection, book: &QueueBook) -> Result<(), Error> {
    conn.execute("DELETE FROM queue_items", [])?;
    for item in book.items() {
        insert_item(conn, &item)?;
    }
    conn.execute("DELETE FROM queue_transitions", [])?;
    for row in book.all_transitions() {
        insert_transition(conn, &row)?;
    }
    Ok(())
}

/// Load the live queue. An empty database is an empty book.
///
/// # Errors
///
/// Returns a SQLite or JSON error.
pub fn load_book(conn: &Connection) -> Result<QueueBook, Error> {
    let items = docs(conn, "SELECT doc FROM queue_items")?;
    let transitions = docs(conn, "SELECT doc FROM queue_transitions ORDER BY id")?;
    let items = parse_each::<QueueItem>(&items)?;
    let transitions = parse_each::<Transition>(&transitions)?;
    Ok(QueueBook::restore(items, transitions))
}

/// Move terminal rows older than `cutoff` into the archive tables.
///
/// # Errors
///
/// Returns a SQLite error.
pub fn archive_terminal(conn: &mut Connection, cutoff: Timestamp) -> Result<Vec<String>, Error> {
    let book = load_book(conn)?;
    let ids = book.retention_ids(cutoff);
    let tx = conn.transaction()?;
    let mut moved = Vec::new();
    for id in ids {
        let key = id.to_string();
        tx.execute(
            "INSERT INTO queue_items_archive (id, doc)
             SELECT id, doc FROM queue_items WHERE id = ?1",
            params![key],
        )?;
        tx.execute(
            "INSERT INTO queue_transitions_archive (id, qitem, doc)
             SELECT id, qitem, doc FROM queue_transitions WHERE qitem = ?1",
            params![key],
        )?;
        tx.execute(
            "DELETE FROM queue_transitions WHERE qitem = ?1",
            params![key],
        )?;
        tx.execute("DELETE FROM queue_items WHERE id = ?1", params![key])?;
        moved.push(key);
    }
    tx.commit()?;
    Ok(moved)
}

/// Store one direct-send receipt. `cursor` is the slice 8 delivery cursor.
///
/// # Errors
///
/// Returns a SQLite error.
pub fn insert_receipt(conn: &Connection, cursor: u64, doc: &str) -> Result<(), Error> {
    conn.execute(
        "INSERT INTO delivery_receipts (cursor, doc) VALUES (?1, ?2)",
        params![i64::try_from(cursor).unwrap_or(i64::MAX), doc],
    )?;
    Ok(())
}

/// Highest receipt cursor, or 0 when the table is empty.
///
/// # Errors
///
/// Returns a SQLite error.
pub fn max_receipt_cursor(conn: &Connection) -> Result<u64, Error> {
    let max: Option<i64> =
        conn.query_row("SELECT MAX(cursor) FROM delivery_receipts", [], |row| {
            row.get(0)
        })?;
    Ok(max.and_then(|value| u64::try_from(value).ok()).unwrap_or(0))
}

fn insert_item(conn: &Connection, item: &QueueItem) -> Result<(), Error> {
    let doc = serde_json::to_string(item).map_err(|err| Error::Event(err.to_string()))?;
    conn.execute(
        "INSERT INTO queue_items (id, state, updated_ms, doc) VALUES (?1, ?2, ?3, ?4)",
        params![
            item.id.to_string(),
            item.state.as_str(),
            item.updated_at.millis(),
            doc
        ],
    )?;
    Ok(())
}

fn insert_transition(conn: &Connection, row: &Transition) -> Result<(), Error> {
    let doc = serde_json::to_string(row).map_err(|err| Error::Event(err.to_string()))?;
    conn.execute(
        "INSERT INTO queue_transitions (id, qitem, doc) VALUES (?1, ?2, ?3)",
        params![
            i64::try_from(row.id).unwrap_or(i64::MAX),
            row.qitem.to_string(),
            doc
        ],
    )?;
    Ok(())
}

fn docs(conn: &Connection, sql: &str) -> Result<Vec<String>, Error> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn parse_each<T: serde::de::DeserializeOwned>(docs: &[String]) -> Result<Vec<T>, Error> {
    let mut out = Vec::with_capacity(docs.len());
    for doc in docs {
        let value = serde_json::from_str(doc).map_err(|err| Error::Event(err.to_string()))?;
        out.push(value);
    }
    Ok(out)
}
