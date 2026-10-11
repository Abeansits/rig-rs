//! Repository round-trip on one `:memory:` connection.

use std::path::PathBuf;

use crewmux_core::{
    DomainEvent, EventLog, Rig, RigRepo, Seat, SeatRepo, SeatState, SessionRepo, Timestamp,
};
use crewmux_store::Store;
use crewmux_types::{ReadinessCode, RigId, RigName, RuntimeKind, SeatId, SeatName, SessionId};

#[test]
fn rig_seat_session_and_events_round_trip() {
    let store = Store::open(PathBuf::from(":memory:").as_path()).expect("open");
    let rig_id = RigId::new();
    let seat_id = SeatId::new();
    let session_id = SessionId::new();
    let now = Timestamp::from_millis(42);
    store
        .write(move |conn| {
            let conn = crewmux_store::Db::new(conn);
            let rig = Rig {
                id: rig_id,
                name: RigName::new("pair").expect("name"),
                created_at: now,
            };
            RigRepo::upsert(&conn, &rig)?;
            let seat = Seat {
                id: seat_id,
                rig_id,
                name: SeatName::new("one").expect("seat"),
                runtime: RuntimeKind::Stub,
                cwd: Some(PathBuf::from("/tmp")),
                model: Some("small".to_owned()),
                state: SeatState::Attention(ReadinessCode::LoginRequired),
                reason: Some("gate".to_owned()),
                tmux_session: Some("one__pair".to_owned()),
                pane_id: Some("%7".to_owned()),
            };
            SeatRepo::upsert(&conn, &seat)?;
            conn.insert(&session_id, &seat_id, "one__pair", Some("%7"), now)?;
            conn.append(&DomainEvent::RigCreated {
                rig: rig_id,
                name: rig.name,
                at: now,
            })?;
            conn.append(&DomainEvent::SeatLaunched {
                seat: seat_id,
                session: session_id,
                at: now,
            })?;
            Ok(())
        })
        .expect("write");
    let (rig, seat, events) = store
        .read(move |conn| {
            let conn = crewmux_store::Db::new(conn);
            let rig = conn.get_by_name(&RigName::new("pair").expect("name"))?;
            let seat = conn.get_by_pane("%7")?;
            let events = EventLog::list(&conn)?;
            Ok((rig, seat, events))
        })
        .expect("read");
    let rig = rig.expect("rig");
    assert_eq!(rig.id, rig_id);
    let seat = seat.expect("seat");
    assert_eq!(
        seat.state,
        SeatState::Attention(ReadinessCode::LoginRequired)
    );
    assert_eq!(seat.runtime, RuntimeKind::Stub);
    assert_eq!(seat.reason.as_deref(), Some("gate"));
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].kind(), "rig_created");
    assert_eq!(events[1].kind(), "seat_launched");
}
