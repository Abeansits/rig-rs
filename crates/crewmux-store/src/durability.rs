//! Checkpoint, delivery, edge, and posture rows.
//!
//! These are ordinary writer-thread writes. SQLite `wal_checkpoint` is not
//! issued here; snapshots use [`crate::Store::backup_maintenance`].

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::Error;

/// Persisted launch posture and resume token for one seat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeatRestoreRow {
    /// Seat id text.
    pub seat_id: String,
    /// `locked`, `floor`, `auto`, or `full_bypass`.
    pub posture: String,
    /// Resume token family, when one was minted.
    pub resume_kind: Option<String>,
    /// Resume token value.
    pub resume_value: Option<String>,
    /// Transcript byte cursor at the last delivery.
    pub transcript_cursor: Option<i64>,
    /// Startup delivery status, when one was recorded.
    pub startup_status: Option<String>,
}

/// A launch-order edge. Only `delegates_to` and `spawned_by` constrain waves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EdgeRow {
    /// Source seat id.
    pub source_id: String,
    /// Target seat id.
    pub target_id: String,
    /// Edge kind.
    pub kind: String,
}

/// One agent checkpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckpointRow {
    /// Checkpoint id.
    pub id: String,
    /// Seat id.
    pub seat_id: String,
    /// Summary.
    pub summary: String,
    /// Current task.
    pub current_task: Option<String>,
    /// Next step.
    pub next_step: Option<String>,
    /// Blocked on.
    pub blocked_on: Option<String>,
    /// JSON array of artifact paths.
    pub key_artifacts: String,
    /// Confidence label.
    pub confidence: Option<String>,
    /// Created, unix millis.
    pub created_ms: i64,
}

/// One direct or startup delivery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectRow {
    /// Delivery id.
    pub id: String,
    /// Rig id.
    pub rig_id: String,
    /// Seat id.
    pub seat_id: String,
    /// `direct` or `startup`.
    pub channel: String,
    /// `sha256` of the normalised body.
    pub fingerprint: String,
    /// Body. Restore never pastes a queue body; this is direct/startup only.
    pub body: String,
    /// Set once the paste landed. Absent means the crash was before the paste.
    pub delivered_at_ms: Option<i64>,
    /// Transcript cursor at delivery.
    pub cursor: Option<i64>,
    /// Transcript ack.
    pub acked: bool,
}

/// Insert or replace the seat's restore row.
///
/// # Errors
///
/// Returns a SQLite error.
pub fn put_seat_restore(conn: &Connection, row: &SeatRestoreRow) -> Result<(), Error> {
    conn.execute(
        "INSERT INTO seat_restore (
            seat_id, posture, resume_kind, resume_value, transcript_cursor, startup_status
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(seat_id) DO UPDATE SET
            posture = excluded.posture,
            resume_kind = excluded.resume_kind,
            resume_value = excluded.resume_value,
            transcript_cursor = excluded.transcript_cursor,
            startup_status = excluded.startup_status",
        params![
            row.seat_id,
            row.posture,
            row.resume_kind,
            row.resume_value,
            row.transcript_cursor,
            row.startup_status,
        ],
    )?;
    Ok(())
}

/// The restore row for one seat.
///
/// # Errors
///
/// Returns a SQLite error.
pub fn load_seat_restore(
    conn: &Connection,
    seat_id: &str,
) -> Result<Option<SeatRestoreRow>, Error> {
    conn.query_row(
        "SELECT seat_id, posture, resume_kind, resume_value, transcript_cursor, startup_status
         FROM seat_restore WHERE seat_id = ?1",
        params![seat_id],
        |row| {
            Ok(SeatRestoreRow {
                seat_id: row.get(0)?,
                posture: row.get(1)?,
                resume_kind: row.get(2)?,
                resume_value: row.get(3)?,
                transcript_cursor: row.get(4)?,
                startup_status: row.get(5)?,
            })
        },
    )
    .optional()
    .map_err(Error::from)
}

/// Record one constraining or informational edge.
///
/// # Errors
///
/// Returns a SQLite error.
pub fn put_edge(conn: &Connection, rig_id: &str, edge: &EdgeRow) -> Result<(), Error> {
    conn.execute(
        "INSERT OR IGNORE INTO seat_edges (rig_id, source_id, target_id, kind) VALUES (?1, ?2, ?3, ?4)",
        params![rig_id, edge.source_id, edge.target_id, edge.kind],
    )?;
    Ok(())
}

/// Edges for one rig.
///
/// # Errors
///
/// Returns a SQLite error.
pub fn list_edges(conn: &Connection, rig_id: &str) -> Result<Vec<EdgeRow>, Error> {
    let mut stmt = conn.prepare(
        "SELECT source_id, target_id, kind FROM seat_edges WHERE rig_id = ?1 ORDER BY kind, source_id, target_id",
    )?;
    let rows = stmt
        .query_map(params![rig_id], |row| {
            Ok(EdgeRow {
                source_id: row.get(0)?,
                target_id: row.get(1)?,
                kind: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Insert one checkpoint.
///
/// # Errors
///
/// Returns a SQLite error.
pub fn insert_checkpoint(conn: &Connection, row: &CheckpointRow) -> Result<(), Error> {
    conn.execute(
        "INSERT INTO checkpoints (
            id, seat_id, summary, current_task, next_step, blocked_on, key_artifacts, confidence, created_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            row.id,
            row.seat_id,
            row.summary,
            row.current_task,
            row.next_step,
            row.blocked_on,
            row.key_artifacts,
            row.confidence,
            row.created_ms,
        ],
    )?;
    Ok(())
}

/// Checkpoints for one seat, oldest first.
///
/// # Errors
///
/// Returns a SQLite error.
pub fn list_checkpoints(conn: &Connection, seat_id: &str) -> Result<Vec<CheckpointRow>, Error> {
    let mut stmt = conn.prepare(
        "SELECT id, seat_id, summary, current_task, next_step, blocked_on, key_artifacts, confidence, created_ms
         FROM checkpoints WHERE seat_id = ?1 ORDER BY created_ms ASC, rowid ASC",
    )?;
    let rows = stmt
        .query_map(params![seat_id], map_checkpoint)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Insert one direct or startup delivery.
///
/// # Errors
///
/// Returns a SQLite error.
pub fn insert_direct(conn: &Connection, row: &DirectRow) -> Result<(), Error> {
    conn.execute(
        "INSERT INTO direct_deliveries (
            id, rig_id, seat_id, channel, fingerprint, body, delivered_at_ms, cursor, acked
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            row.id,
            row.rig_id,
            row.seat_id,
            row.channel,
            row.fingerprint,
            row.body,
            row.delivered_at_ms,
            row.cursor,
            i64::from(row.acked),
        ],
    )?;
    Ok(())
}

/// Deliveries for one rig, oldest first.
///
/// # Errors
///
/// Returns a SQLite error.
pub fn list_direct(conn: &Connection, rig_id: &str) -> Result<Vec<DirectRow>, Error> {
    let mut stmt = conn.prepare(
        "SELECT id, rig_id, seat_id, channel, fingerprint, body, delivered_at_ms, cursor, acked
         FROM direct_deliveries WHERE rig_id = ?1 ORDER BY rowid ASC",
    )?;
    let rows = stmt
        .query_map(params![rig_id], |row| {
            Ok(DirectRow {
                id: row.get(0)?,
                rig_id: row.get(1)?,
                seat_id: row.get(2)?,
                channel: row.get(3)?,
                fingerprint: row.get(4)?,
                body: row.get(5)?,
                delivered_at_ms: row.get(6)?,
                cursor: row.get(7)?,
                acked: row.get::<_, i64>(8)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Mark a delivery acked. The row stays; restore will not plan it again.
///
/// # Errors
///
/// Returns a SQLite error.
pub fn set_direct_acked(conn: &Connection, id: &str) -> Result<(), Error> {
    conn.execute(
        "UPDATE direct_deliveries SET acked = 1 WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

/// Record that a crashed-before-paste delivery landed.
///
/// # Errors
///
/// Returns a SQLite error.
pub fn set_direct_delivered(conn: &Connection, id: &str, at_ms: i64) -> Result<(), Error> {
    conn.execute(
        "UPDATE direct_deliveries SET delivered_at_ms = ?2 WHERE id = ?1",
        params![id, at_ms],
    )?;
    Ok(())
}

/// Remember that this rig is the kernel rig.
///
/// # Errors
///
/// Returns a SQLite error.
pub fn mark_kernel(conn: &Connection, rig_id: &str) -> Result<(), Error> {
    conn.execute(
        "INSERT OR IGNORE INTO rig_kernel (rig_id) VALUES (?1)",
        params![rig_id],
    )?;
    Ok(())
}

/// Rig ids flagged as kernel.
///
/// # Errors
///
/// Returns a SQLite error.
pub fn rigs_kernel(conn: &Connection) -> Result<Vec<String>, Error> {
    let mut stmt = conn.prepare("SELECT rig_id FROM rig_kernel ORDER BY rig_id")?;
    let rows = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(rows)
}

fn map_checkpoint(row: &rusqlite::Row<'_>) -> rusqlite::Result<CheckpointRow> {
    Ok(CheckpointRow {
        id: row.get(0)?,
        seat_id: row.get(1)?,
        summary: row.get(2)?,
        current_task: row.get(3)?,
        next_step: row.get(4)?,
        blocked_on: row.get(5)?,
        key_artifacts: row.get(6)?,
        confidence: row.get(7)?,
        created_ms: row.get(8)?,
    })
}
