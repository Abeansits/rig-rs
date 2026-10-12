//! Copy one rig out of a snapshot database into the live store.

use std::collections::HashSet;
use std::path::Path;

use crewmux_messaging::{QueueBook, QueueItem};
use crewmux_types::QItemId;
use rusqlite::{Connection, OptionalExtension, params};

use crate::durability::{
    insert_checkpoint, insert_direct, list_checkpoints, list_direct, list_edges, load_seat_restore,
    put_edge, put_seat_restore,
};
use crate::error::Error;
use crate::queue::{load_book, overwrite_book};

/// Replace the live rows for `rig_name` with the rows in `snap_path`.
///
/// Seats keep their live session fields. Cwd, resume, edges, deliveries,
/// checkpoints, and that rig's queue rows come from the snapshot.
///
/// # Errors
///
/// Returns [`Error`] when the snapshot cannot be read or a write fails.
pub fn import_rig(live: &mut Connection, snap_path: &Path, rig_name: &str) -> Result<(), Error> {
    let snap = Connection::open_with_flags(snap_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let rig_id = rig_id(&snap, rig_name)?;
    let tx = live.transaction()?;
    copy_rig_row(&tx, &snap, &rig_id)?;
    let seats = seat_ids(&snap, &rig_id)?;
    for id in &seats {
        copy_seat(&tx, &snap, id)?;
        if let Some(row) = load_seat_restore(&snap, id)? {
            put_seat_restore(&tx, &row)?;
        }
    }
    copy_edges(&tx, &snap, &rig_id)?;
    copy_direct(&tx, &snap, &rig_id)?;
    copy_checkpoints(&tx, &snap, &seats)?;
    let book = merged_queue(&tx, &snap, rig_name)?;
    overwrite_book(&tx, &book)?;
    tx.commit()?;
    Ok(())
}

fn rig_id(snap: &Connection, name: &str) -> Result<String, Error> {
    let id = snap
        .query_row(
            "SELECT id FROM rigs WHERE name = ?1",
            params![name],
            |row| row.get(0),
        )
        .optional()?;
    id.ok_or_else(|| Error::Event(format!("snapshot has no rig {name}")))
}

fn copy_rig_row(live: &Connection, snap: &Connection, rig_id: &str) -> Result<(), Error> {
    let (name, created): (String, i64) = snap.query_row(
        "SELECT name, created_ms FROM rigs WHERE id = ?1",
        params![rig_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    live.execute(
        "INSERT INTO rigs (id, name, created_ms) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET name = excluded.name",
        params![rig_id, name, created],
    )?;
    Ok(())
}

fn seat_ids(snap: &Connection, rig_id: &str) -> Result<Vec<String>, Error> {
    let mut stmt = snap.prepare("SELECT id FROM seats WHERE rig_id = ?1 ORDER BY name")?;
    let rows = stmt
        .query_map(params![rig_id], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(rows)
}

fn copy_seat(live: &Connection, snap: &Connection, id: &str) -> Result<(), Error> {
    let row = snap.query_row(
        "SELECT rig_id, name, runtime, cwd, model FROM seats WHERE id = ?1",
        params![id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        },
    )?;
    live.execute(
        "INSERT INTO seats (
            id, rig_id, name, runtime, cwd, model, state, attention_code, reason, tmux_session, pane_id
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'planned', NULL, NULL, NULL, NULL)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            runtime = excluded.runtime,
            cwd = excluded.cwd,
            model = excluded.model",
        params![id, row.0, row.1, row.2, row.3, row.4],
    )?;
    Ok(())
}

fn copy_edges(live: &Connection, snap: &Connection, rig_id: &str) -> Result<(), Error> {
    live.execute("DELETE FROM seat_edges WHERE rig_id = ?1", params![rig_id])?;
    for edge in list_edges(snap, rig_id)? {
        put_edge(live, rig_id, &edge)?;
    }
    Ok(())
}

fn copy_direct(live: &Connection, snap: &Connection, rig_id: &str) -> Result<(), Error> {
    live.execute(
        "DELETE FROM direct_deliveries WHERE rig_id = ?1",
        params![rig_id],
    )?;
    for row in list_direct(snap, rig_id)? {
        insert_direct(live, &row)?;
    }
    Ok(())
}

fn copy_checkpoints(live: &Connection, snap: &Connection, seats: &[String]) -> Result<(), Error> {
    for id in seats {
        live.execute("DELETE FROM checkpoints WHERE seat_id = ?1", params![id])?;
        for row in list_checkpoints(snap, id)? {
            insert_checkpoint(live, &row)?;
        }
    }
    Ok(())
}

fn merged_queue(live: &Connection, snap: &Connection, rig: &str) -> Result<QueueBook, Error> {
    let live_book = load_book(live)?;
    let snap_book = load_book(snap)?;
    let suffix = format!("@{rig}");
    let belongs = |item: &QueueItem| item.destination.as_str().ends_with(suffix.as_str());
    let mut items = Vec::new();
    let mut kept = HashSet::new();
    for item in live_book.items() {
        if belongs(&item) {
            continue;
        }
        kept.insert(item.id.clone());
        items.push(item);
    }
    let mut imported = HashSet::new();
    for item in snap_book.items() {
        if !belongs(&item) {
            continue;
        }
        imported.insert(item.id.clone());
        items.push(item);
    }
    let transitions = queue_transitions(&live_book, &snap_book, &kept, &imported);
    Ok(QueueBook::restore(items, transitions))
}

fn queue_transitions(
    live_book: &QueueBook,
    snap_book: &QueueBook,
    kept: &HashSet<QItemId>,
    imported: &HashSet<QItemId>,
) -> Vec<crewmux_messaging::Transition> {
    let mut transitions = Vec::new();
    for row in live_book.all_transitions() {
        if kept.contains(&row.qitem) {
            transitions.push(row);
        }
    }
    for row in snap_book.all_transitions() {
        if imported.contains(&row.qitem) {
            transitions.push(row);
        }
    }
    transitions
}
