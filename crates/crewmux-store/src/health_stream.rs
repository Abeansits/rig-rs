// Derived from openrig packages/daemon/src/domain/stream-store.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Stream items and the classification attempt ledger.

use crewmux_health::{MAX_ELIGIBLE_PAGE, StreamItem};
use rusqlite::{Connection, OptionalExtension, params};
use ulid::Ulid;

use crate::error::Error;

/// What `POST /api/stream/emit` stores.
#[derive(Clone, Debug)]
pub struct NewStream {
    /// Caller id. A new ULID is minted when this is empty.
    pub id: Option<String>,
    /// Kind. Empty becomes `note`.
    pub kind: String,
    /// Required body.
    pub body: String,
    /// Emitting session.
    pub source: String,
    /// Emit time, milliseconds.
    pub at_ms: i64,
}

/// Emit result. `created` is false when `id` was already stored.
#[derive(Clone, Debug)]
pub struct Emitted {
    /// The stored item.
    pub item: StreamItem,
    /// True when this call inserted the row.
    pub created: bool,
}

/// Insert `item`, or return the existing row when the id is already present.
///
/// # Errors
///
/// Returns [`Error`] when the body is empty or the write fails.
pub fn stream_emit(conn: &Connection, item: &NewStream) -> Result<Emitted, Error> {
    if item.body.is_empty() {
        return Err(Error::Health(crewmux_health::Error::Rejected(
            "body is required".into(),
        )));
    }
    if let Some(id) = item.id.as_deref().filter(|id| !id.is_empty())
        && let Some(existing) = stream_get(conn, id)?
    {
        if existing.body != item.body {
            return Err(Error::Health(crewmux_health::Error::Rejected(
                "stream item id already exists with a different body".into(),
            )));
        }
        return Ok(Emitted {
            item: existing,
            created: false,
        });
    }
    let id = item
        .id
        .clone()
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| Ulid::generate().to_string());
    let kind = if item.kind.is_empty() {
        "note".to_owned()
    } else {
        item.kind.clone()
    };
    conn.execute(
        "INSERT INTO stream_items (id, kind, body, labels, archived, source, emitted_at_ms)
         VALUES (?1, ?2, ?3, '{}', 0, ?4, ?5)",
        params![id, kind, item.body, item.source, item.at_ms],
    )?;
    let stored = stream_get(conn, &id)?.ok_or_else(|| {
        Error::Health(crewmux_health::Error::Rejected(
            "stream insert vanished".into(),
        ))
    })?;
    Ok(Emitted {
        item: stored,
        created: true,
    })
}

/// One item, including archived rows.
///
/// # Errors
///
/// Returns [`Error`] when the row cannot be read.
pub fn stream_get(conn: &Connection, id: &str) -> Result<Option<StreamItem>, Error> {
    conn.query_row(
        "SELECT seq, id, kind, body, labels, archived, source, emitted_at_ms
         FROM stream_items WHERE id = ?1",
        params![id],
        map_item,
    )
    .optional()
    .map_err(Into::into)
}

/// Chronological page. Archived rows are excluded unless `include_archived`.
///
/// # Errors
///
/// Returns [`Error`] when the query fails.
pub fn stream_list(
    conn: &Connection,
    limit: u32,
    include_archived: bool,
) -> Result<Vec<StreamItem>, Error> {
    let limit = i64::from(limit.clamp(1, 200));
    let sql = if include_archived {
        "SELECT seq, id, kind, body, labels, archived, source, emitted_at_ms
         FROM stream_items ORDER BY seq ASC LIMIT ?1"
    } else {
        "SELECT seq, id, kind, body, labels, archived, source, emitted_at_ms
         FROM stream_items WHERE archived = 0 ORDER BY seq ASC LIMIT ?1"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![limit], map_item)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Archive one active item.
///
/// # Errors
///
/// Returns [`Error::Health`] when the id is missing or already archived.
pub fn stream_archive(conn: &Connection, id: &str) -> Result<StreamItem, Error> {
    let changed = conn.execute(
        "UPDATE stream_items SET archived = 1 WHERE id = ?1 AND archived = 0",
        params![id],
    )?;
    if changed == 0 {
        return Err(Error::Health(crewmux_health::Error::Rejected(
            "stream item not found or already archived".into(),
        )));
    }
    stream_get(conn, id)?.ok_or_else(|| {
        Error::Health(crewmux_health::Error::Rejected(
            "stream item not found or already archived".into(),
        ))
    })
}

/// How many classification attempts are stored for `stream_id`.
///
/// # Errors
///
/// Returns [`Error`] when the count cannot be read.
pub fn attempt_count(conn: &Connection, stream_id: &str) -> Result<u32, Error> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM classification_attempts WHERE stream_id = ?1",
        params![stream_id],
        |row| row.get(0),
    )?;
    u32::try_from(count).map_err(|_| {
        Error::Health(crewmux_health::Error::Classify(
            "attempt count is out of range".into(),
        ))
    })
}

/// Append one validated decision. The page cap is [`MAX_ELIGIBLE_PAGE`].
///
/// # Errors
///
/// Returns [`Error::Health`] when the stream already has 100 attempts.
pub fn attempt_insert(
    conn: &Connection,
    stream_id: &str,
    lease_id: &str,
    holder: &str,
    decision: &str,
    now_ms: i64,
) -> Result<(), Error> {
    let count = attempt_count(conn, stream_id)?;
    if count >= MAX_ELIGIBLE_PAGE {
        return Err(Error::Health(crewmux_health::Error::Classify(format!(
            "eligible page must be 1..={MAX_ELIGIBLE_PAGE}"
        ))));
    }
    conn.execute(
        "INSERT INTO classification_attempts (stream_id, lease_id, holder, decision, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![stream_id, lease_id, holder, decision, now_ms],
    )?;
    Ok(())
}

fn map_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<StreamItem> {
    let labels: String = row.get(4)?;
    let archived: i64 = row.get(5)?;
    Ok(StreamItem {
        seq: row.get(0)?,
        id: row.get(1)?,
        kind: row.get(2)?,
        body: row.get(3)?,
        labels: serde_json::from_str(&labels).unwrap_or_else(|_| serde_json::json!({})),
        archived: archived != 0,
        source: row.get(6)?,
        emitted_at_ms: row.get(7)?,
    })
}
