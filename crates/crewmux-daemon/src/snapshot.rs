//! Snapshot files under `$RIG_HOME/snapshots/<rig>/<id>/`.
//!
//! The backup uses [`crewmux_store::Store::backup_maintenance`], never the writer.

use std::path::PathBuf;
use std::sync::Arc;

use crewmux_core::{RigRepo, SeatRepo};
use crewmux_durability::{
    DeliveryChannel, InFlightDelivery, SeatSnapshot, SnapshotManifest, StoredResume, sha256_hex,
    verify_snapshot,
};
use crewmux_messaging::unconfirmed_ids;
use crewmux_store::MIGRATIONS;
use crewmux_types::{DeliveryId, SeatId, SnapshotId};
use serde_json::{Value, json};

use crate::db::read;
use crate::error::Error;
use crate::resume::recorded_posture;
use crate::state::{App, join_err, now_ms};

/// Create a snapshot of one rig.
///
/// # Errors
///
/// Returns [`Error`] when the rig is missing or the backup fails.
pub(crate) async fn create(app: &App, rig_name: &str) -> Result<Value, Error> {
    let (rig_id, seats) = load_rig(app, rig_name).await?;
    let id = SnapshotId::new();
    let dir = snap_dir(&app.home, rig_name, &id.to_string());
    let db_path = dir.join("rig.db");
    backup(app, db_path.clone()).await?;
    let bytes = read_file(db_path.clone()).await?;
    let sha = sha256_hex(&bytes);
    let schema = schema_of(&db_path).await?;
    let manifest = build_manifest(app, &rig_id, rig_name, id, schema, sha, &seats).await?;
    let path = dir.join("manifest.json");
    let text =
        serde_json::to_string_pretty(&manifest).map_err(|err| Error::Message(err.to_string()))?;
    write_file(path, text).await?;
    serde_json::to_value(&manifest).map_err(|err| Error::Message(err.to_string()))
}

/// List snapshot ids for a rig.
///
/// # Errors
///
/// Returns [`Error`] when the directory cannot be read.
pub(crate) async fn list(app: &App, rig_name: &str) -> Result<Value, Error> {
    let dir = app.home.join("snapshots").join(rig_name);
    let ids = tokio::task::spawn_blocking(move || read_ids(&dir))
        .await
        .map_err(join_err)??;
    Ok(json!({"rig": rig_name, "snapshots": ids}))
}

/// Show one snapshot, refusing a bad hash or a newer schema.
///
/// # Errors
///
/// Returns [`Error`] when the snapshot is missing, corrupt, or too new.
pub(crate) async fn show(app: &App, rig_name: &str, id: &str) -> Result<Value, Error> {
    let manifest = load_verified(app, rig_name, id).await?;
    serde_json::to_value(&manifest).map_err(|err| Error::Message(err.to_string()))
}

/// Verify a stored snapshot. Restore calls this before relaunching.
///
/// # Errors
///
/// Returns [`Error`] when the snapshot is missing, corrupt, or too new.
pub(crate) async fn verify_stored(app: &App, rig_name: &str, id: &str) -> Result<(), Error> {
    load_verified(app, rig_name, id).await.map(|_| ())
}

async fn load_verified(app: &App, rig_name: &str, id: &str) -> Result<SnapshotManifest, Error> {
    let dir = snap_dir(&app.home, rig_name, id);
    let manifest_path = dir.join("manifest.json");
    let db_path = dir.join("rig.db");
    let text = read_file(manifest_path).await?;
    let manifest: SnapshotManifest = serde_json::from_slice(&text)
        .map_err(|err| Error::Message(format!("snapshot manifest: {err}")))?;
    let bytes = read_file(db_path.clone()).await?;
    let file_schema = schema_of(&db_path).await?;
    let supported = u32::try_from(MIGRATIONS.len()).unwrap_or(u32::MAX);
    verify_snapshot(&manifest, &sha256_hex(&bytes), file_schema, supported)
        .map_err(|err| Error::Message(err.to_string()))?;
    Ok(manifest)
}

#[expect(
    clippy::too_many_arguments,
    reason = "manifest fields are assembled from the backup and the live rows"
)]
async fn build_manifest(
    app: &App,
    rig_id: &crewmux_types::RigId,
    rig_name: &str,
    id: SnapshotId,
    schema: u32,
    sha: String,
    seats: &[crewmux_core::Seat],
) -> Result<SnapshotManifest, Error> {
    let mut captured = Vec::new();
    for seat in seats {
        captured.push(seat_snap(app, seat).await?);
    }
    let in_flight = in_flight(app, rig_id).await?;
    Ok(SnapshotManifest {
        id,
        rig: *rig_id,
        rig_name: rig_name.to_owned(),
        created_at: i64::try_from(now_ms()).unwrap_or(0),
        schema_version: schema,
        sha256: sha,
        seats: captured,
        in_flight,
    })
}

async fn seat_snap(app: &App, seat: &crewmux_core::Seat) -> Result<SeatSnapshot, Error> {
    let posture = recorded_posture(app, seat).await?;
    let id = seat.id.to_string();
    let store = Arc::clone(&app.store);
    let row = tokio::task::spawn_blocking(move || {
        store.read(move |conn| crewmux_store::load_seat_restore(conn, &id))
    })
    .await
    .map_err(join_err)??;
    let resume = row
        .as_ref()
        .and_then(|row| match (&row.resume_kind, &row.resume_value) {
            (Some(kind), Some(value)) => Some(StoredResume {
                kind: kind.clone(),
                value: value.clone(),
            }),
            _ => None,
        });
    let cursor = row.and_then(|row| {
        row.transcript_cursor
            .and_then(|value| u64::try_from(value).ok())
    });
    Ok(SeatSnapshot {
        seat: seat.id,
        name: seat.name.as_str().to_owned(),
        runtime: seat.runtime,
        resume,
        transcript_cursor: cursor,
        posture,
        startup_status: None,
    })
}

async fn in_flight(
    app: &App,
    rig_id: &crewmux_types::RigId,
) -> Result<Vec<InFlightDelivery>, Error> {
    let id = rig_id.to_string();
    let store = Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || {
        store.read(move |conn| {
            let book = crewmux_store::load_book(conn)?;
            let mut items = Vec::new();
            let pending = unconfirmed_ids(&book.items());
            for item in book.items() {
                if pending.iter().any(|id| id == &item.id) {
                    items.push(InFlightDelivery::QueueRow {
                        qitem: item.id,
                        state: item.state,
                    });
                }
            }
            for row in crewmux_store::list_direct(conn, &id)? {
                if row.acked {
                    continue;
                }
                items.push(direct_item(row)?);
            }
            Ok(items)
        })
    })
    .await
    .map_err(join_err)?
    .map_err(Error::from)
}

fn direct_item(row: crewmux_store::DirectRow) -> Result<InFlightDelivery, crewmux_store::Error> {
    let id = DeliveryId::new(row.id).map_err(|err| crewmux_store::Error::Event(err.to_string()))?;
    let seat = row
        .seat_id
        .parse::<SeatId>()
        .map_err(|err| crewmux_store::Error::Event(err.to_string()))?;
    Ok(InFlightDelivery::Direct {
        id,
        seat,
        fingerprint: row.fingerprint,
        body: row.body,
        delivered_at: row.delivered_at_ms,
        cursor: row.cursor.and_then(|value| u64::try_from(value).ok()),
        channel: if row.channel == "startup" {
            DeliveryChannel::Startup
        } else {
            DeliveryChannel::Direct
        },
    })
}

pub(crate) async fn load_rig(
    app: &App,
    rig_name: &str,
) -> Result<(crewmux_types::RigId, Vec<crewmux_core::Seat>), Error> {
    let name =
        crewmux_types::RigName::new(rig_name).map_err(|err| Error::Message(err.to_string()))?;
    read(&app.store, move |conn| {
        let rig = conn
            .get_by_name(&name)?
            .ok_or_else(|| crewmux_core::Error::RigNotFound(name.to_string()))?;
        let seats = conn.list_by_rig(&rig.id)?;
        Ok((rig.id, seats))
    })
    .await
}

fn snap_dir(home: &std::path::Path, rig: &str, id: &str) -> PathBuf {
    home.join("snapshots").join(rig).join(id)
}

async fn backup(app: &App, dest: PathBuf) -> Result<(), Error> {
    let store = Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || store.backup_maintenance(&dest, || {}))
        .await
        .map_err(join_err)?
        .map_err(Error::from)
}

async fn schema_of(path: &std::path::Path) -> Result<u32, Error> {
    let path = path.to_path_buf();
    let version =
        tokio::task::spawn_blocking(move || crewmux_store::Store::file_user_version(&path))
            .await
            .map_err(join_err)??;
    u32::try_from(version).map_err(|_| Error::Message("schema version does not fit".to_owned()))
}

async fn read_file(path: PathBuf) -> Result<Vec<u8>, Error> {
    tokio::task::spawn_blocking(move || std::fs::read(&path).map_err(Error::from))
        .await
        .map_err(join_err)?
}

async fn write_file(path: PathBuf, text: String) -> Result<(), Error> {
    tokio::task::spawn_blocking(move || std::fs::write(&path, text).map_err(Error::from))
        .await
        .map_err(join_err)?
}

fn read_ids(dir: &std::path::Path) -> Result<Vec<String>, Error> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    let mut ids = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            ids.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    ids.sort();
    Ok(ids)
}
