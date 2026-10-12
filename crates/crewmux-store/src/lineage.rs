//! Pane process recorded at launch, checked again before a send.

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::Error;

/// The process a seat was launched as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneLineage {
    /// Seat id.
    pub seat_id: String,
    /// `pane_pid` at launch.
    pub pid: u32,
    /// Foreground command at launch.
    pub command: String,
    /// Session or resume token, when the launch named one.
    pub session_id: Option<String>,
    /// Runtime token (`claude-code`, `codex`, `stub`, …).
    pub runtime: String,
    /// Process start token from `/proc` start ticks or `ps` lstart.
    ///
    /// `None` when the launch could not read it. A later check then cannot
    /// treat a missing argv session as the same process.
    pub started_ticks: Option<u64>,
}

/// Insert or replace the launch lineage for one seat.
///
/// # Errors
///
/// Returns [`Error`] when the write fails.
pub fn lineage_put(conn: &Connection, row: &PaneLineage) -> Result<(), Error> {
    let pid = i64::from(row.pid);
    let started = match row.started_ticks {
        Some(ticks) => Some(i64::try_from(ticks).map_err(|_| {
            Error::Health(crewmux_health::Error::Rejected(
                "pane lineage start tick is out of range".into(),
            ))
        })?),
        None => None,
    };
    conn.execute(
        "INSERT INTO pane_lineage (seat_id, pid, command, session_id, runtime, started_ticks)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(seat_id) DO UPDATE SET
            pid = excluded.pid,
            command = excluded.command,
            session_id = excluded.session_id,
            runtime = excluded.runtime,
            started_ticks = excluded.started_ticks",
        params![
            row.seat_id,
            pid,
            row.command,
            row.session_id,
            row.runtime,
            started
        ],
    )?;
    Ok(())
}

/// The lineage recorded for `seat_id`.
///
/// # Errors
///
/// Returns [`Error`] when the row cannot be read.
pub fn lineage_get(conn: &Connection, seat_id: &str) -> Result<Option<PaneLineage>, Error> {
    let found = conn
        .query_row(
            "SELECT seat_id, pid, command, session_id, runtime, started_ticks
             FROM pane_lineage WHERE seat_id = ?1",
            params![seat_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                ))
            },
        )
        .optional()?;
    let Some((seat_id, pid, command, session_id, runtime, started)) = found else {
        return Ok(None);
    };
    let pid = u32::try_from(pid).map_err(|_| {
        Error::Health(crewmux_health::Error::Rejected(
            "pane lineage pid is out of range".into(),
        ))
    })?;
    let started_ticks = match started {
        Some(ticks) => Some(u64::try_from(ticks).map_err(|_| {
            Error::Health(crewmux_health::Error::Rejected(
                "pane lineage start tick is out of range".into(),
            ))
        })?),
        None => None,
    };
    Ok(Some(PaneLineage {
        seat_id,
        pid,
        command,
        session_id,
        runtime,
        started_ticks,
    }))
}
