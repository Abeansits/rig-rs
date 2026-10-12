//! Checkpoint table, foreign keys, and cascade.
#![expect(
    clippy::expect_used,
    reason = "setup failures abort the integration test"
)]

use std::path::PathBuf;

use crewmux_core::{Rig, RigRepo, Seat, SeatRepo, SeatState, Timestamp};
use crewmux_store::{CheckpointRow, Store};
use crewmux_types::{RigId, RigName, RuntimeKind, SeatId, SeatName};

fn open() -> Store {
    Store::open(PathBuf::from(":memory:").as_path()).expect("open")
}

fn seed(store: &Store) -> (RigId, SeatId) {
    let rig_id = RigId::new();
    let seat_id = SeatId::new();
    let now = Timestamp::from_millis(1);
    store
        .write(move |conn| {
            let db = crewmux_store::Db::new(conn);
            RigRepo::upsert(
                &db,
                &Rig {
                    id: rig_id,
                    name: RigName::new("pair").expect("name"),
                    created_at: now,
                },
            )?;
            SeatRepo::upsert(
                &db,
                &Seat {
                    id: seat_id,
                    rig_id,
                    name: SeatName::new("one").expect("seat"),
                    runtime: RuntimeKind::Stub,
                    cwd: Some(PathBuf::from("/tmp")),
                    model: None,
                    state: SeatState::Ready,
                    reason: None,
                    tmux_session: None,
                    pane_id: None,
                },
            )?;
            Ok(())
        })
        .expect("seed");
    (rig_id, seat_id)
}

fn row(seat: &SeatId, id: &str, created_ms: i64, artifacts: &str) -> CheckpointRow {
    CheckpointRow {
        id: id.to_owned(),
        seat_id: seat.to_string(),
        summary: "shipped".to_owned(),
        current_task: Some("slice".to_owned()),
        next_step: Some("restore".to_owned()),
        blocked_on: None,
        key_artifacts: artifacts.to_owned(),
        confidence: Some("high".to_owned()),
        created_ms,
    }
}

#[test]
fn creates_checkpoints_table() {
    let store = open();
    let names: Vec<String> = store
        .read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'checkpoints'",
            )?;
            let rows = stmt
                .query_map([], |row| row.get(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .expect("names");
    assert_eq!(names, vec!["checkpoints".to_owned()]);
}

#[test]
fn can_insert_checkpoint_for_a_node() {
    let store = open();
    let (_, seat) = seed(&store);
    store
        .write(move |conn| {
            crewmux_store::insert_checkpoint(conn, &row(&seat, "cp-1", 10, "[\"notes.md\"]"))
        })
        .expect("insert");
    let loaded = store
        .read(move |conn| crewmux_store::list_checkpoints(conn, &seat.to_string()))
        .expect("list");
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].key_artifacts, "[\"notes.md\"]");
    assert_eq!(loaded[0].summary, "shipped");
}

#[test]
fn newest_checkpoint_is_the_latest_timestamp() {
    let store = open();
    let (_, seat) = seed(&store);
    store
        .write(move |conn| {
            crewmux_store::insert_checkpoint(conn, &row(&seat, "cp-old", 10, "[]"))?;
            crewmux_store::insert_checkpoint(conn, &row(&seat, "cp-new", 20, "[]"))?;
            Ok(())
        })
        .expect("insert");
    let loaded = store
        .read(move |conn| crewmux_store::list_checkpoints(conn, &seat.to_string()))
        .expect("list");
    assert_eq!(loaded.last().map(|row| row.id.as_str()), Some("cp-new"));
    let empty = store
        .read(|conn| crewmux_store::list_checkpoints(conn, "missing"))
        .expect("empty");
    assert_eq!(empty, Vec::<crewmux_store::CheckpointRow>::new());
}

#[test]
fn fk_enforced_checkpoint_with_invalid_node_id_rejected() {
    let store = open();
    let _ = seed(&store);
    let err = store
        .write(|conn| {
            crewmux_store::insert_checkpoint(conn, &row(&SeatId::new(), "cp-bad", 1, "[]"))
        })
        .expect_err("fk");
    let text = err.to_string();
    assert!(
        text.contains("FOREIGN KEY") || text.contains("foreign key"),
        "{text}"
    );
}

#[test]
fn cascade_deleting_node_removes_its_checkpoints() {
    let store = open();
    let (_, seat) = seed(&store);
    let seat_text = seat.to_string();
    store
        .write({
            let seat_text = seat_text.clone();
            move |conn| {
                crewmux_store::insert_checkpoint(conn, &row(&seat, "cp-1", 10, "[]"))?;
                conn.execute("DELETE FROM seats WHERE id = ?1", [seat_text])?;
                Ok(())
            }
        })
        .expect("delete");
    let loaded = store
        .read(move |conn| crewmux_store::list_checkpoints(conn, &seat_text))
        .expect("list");
    assert_eq!(loaded, Vec::<crewmux_store::CheckpointRow>::new());
}
