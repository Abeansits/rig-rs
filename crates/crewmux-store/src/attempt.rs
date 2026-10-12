//! Persisted compaction attempts. The document is the health crate's JSON.

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::Error;

/// One attempt row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptRow {
    /// Attempt id.
    pub id: String,
    /// Seat key.
    pub seat_key: String,
    /// Unix milliseconds.
    pub updated_ms: i64,
    /// JSON document.
    pub doc: String,
}

/// Insert or replace one attempt.
///
/// # Errors
///
/// Returns [`Error::Sqlite`] when the write fails.
pub fn upsert_attempt(conn: &Connection, row: &AttemptRow) -> Result<(), Error> {
    conn.execute(
        "INSERT INTO compaction_attempts (id, seat_key, updated_ms, doc) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET seat_key = excluded.seat_key, updated_ms = excluded.updated_ms, doc = excluded.doc",
        params![row.id, row.seat_key, row.updated_ms, row.doc],
    )?;
    Ok(())
}

/// Delete one attempt.
///
/// # Errors
///
/// Returns [`Error::Sqlite`] when the delete fails.
pub fn delete_attempt(conn: &Connection, id: &str) -> Result<(), Error> {
    conn.execute("DELETE FROM compaction_attempts WHERE id = ?1", params![id])?;
    Ok(())
}

/// Every attempt, oldest update first.
///
/// # Errors
///
/// Returns [`Error::Sqlite`] when the query fails.
pub fn list_attempts(conn: &Connection) -> Result<Vec<AttemptRow>, Error> {
    let mut statement = conn.prepare(
        "SELECT id, seat_key, updated_ms, doc FROM compaction_attempts ORDER BY updated_ms, id",
    )?;
    let rows = statement.query_map([], read_row)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// One attempt by id.
///
/// # Errors
///
/// Returns [`Error::Sqlite`] when the query fails.
pub fn get_attempt(conn: &Connection, id: &str) -> Result<Option<AttemptRow>, Error> {
    conn.query_row(
        "SELECT id, seat_key, updated_ms, doc FROM compaction_attempts WHERE id = ?1",
        params![id],
        read_row,
    )
    .optional()
    .map_err(Error::from)
}

fn read_row(row: &rusqlite::Row<'_>) -> Result<AttemptRow, rusqlite::Error> {
    Ok(AttemptRow {
        id: row.get(0)?,
        seat_key: row.get(1)?,
        updated_ms: row.get(2)?,
        doc: row.get(3)?,
    })
}
