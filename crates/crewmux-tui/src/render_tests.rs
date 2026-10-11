//! Insta goldens at 120×40 and 80×24.

use std::time::Instant;

use crewmux_api::{
    PodName, ReadinessCode, RigId, RigName, RuntimeKind, SeatId, SeatName, SeatState, SeatView,
    SessionId, Snapshot,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::state::{RowFlash, RowKey, Size, State};
use crate::view::view;

fn seat(id: &str, name: &str, state: SeatState) -> SeatView {
    SeatView {
        id: SeatId::parse(id).expect("seat"),
        name: SeatName::parse(name).expect("name"),
        rig_id: RigId::parse("rig-1").expect("rig"),
        rig_name: RigName::parse("demo").expect("rig name"),
        pod: PodName::parse("pod").expect("pod"),
        state,
        runtime: RuntimeKind::Stub,
        session: Some(SessionId::parse("sess").expect("session")),
        spec: Some("demo".to_owned()),
    }
}

fn paint(state: &State) -> String {
    let mut terminal =
        Terminal::new(TestBackend::new(state.size.cols, state.size.rows)).expect("backend");
    terminal
        .draw(|frame| {
            let hits = view(state, frame);
            assert!(hits.targets.len() < 10_000);
        })
        .expect("draw");
    terminal.backend().to_string()
}

fn sized(cols: u16, rows: u16, seats: Vec<SeatView>) -> State {
    let mut state = State::new(Size::new(cols, rows), Instant::now());
    state.snapshot = Snapshot {
        seats,
        health: None,
        activity: vec![],
    };
    state
}

#[test]
fn empty_instance_120x40() {
    insta::assert_snapshot!(paint(&sized(120, 40, vec![])));
}

#[test]
fn empty_instance_80x24() {
    insta::assert_snapshot!(paint(&sized(80, 24, vec![])));
}

#[test]
fn two_stub_seats_120x40() {
    let seats = vec![
        seat("s1", "alpha", SeatState::Ready),
        seat("s2", "beta", SeatState::Ready),
    ];
    insta::assert_snapshot!(paint(&sized(120, 40, seats)));
}

#[test]
fn two_stub_seats_80x24() {
    let seats = vec![
        seat("s1", "alpha", SeatState::Ready),
        seat("s2", "beta", SeatState::Ready),
    ];
    insta::assert_snapshot!(paint(&sized(80, 24, seats)));
}

#[test]
fn attention_login_required_120x40() {
    let seats = vec![seat(
        "s1",
        "alpha",
        SeatState::Attention(ReadinessCode::LoginRequired),
    )];
    insta::assert_snapshot!(paint(&sized(120, 40, seats)));
}

#[test]
fn attention_login_required_80x24() {
    let seats = vec![seat(
        "s1",
        "alpha",
        SeatState::Attention(ReadinessCode::LoginRequired),
    )];
    insta::assert_snapshot!(paint(&sized(80, 24, seats)));
}

#[test]
fn dead_seat_120x40() {
    insta::assert_snapshot!(paint(&sized(
        120,
        40,
        vec![seat("s1", "alpha", SeatState::Dead)]
    )));
}

#[test]
fn dead_seat_80x24() {
    insta::assert_snapshot!(paint(&sized(
        80,
        24,
        vec![seat("s1", "alpha", SeatState::Dead)]
    )));
}

#[test]
fn row_flash_120x40() {
    let mut state = sized(120, 40, vec![seat("s1", "alpha", SeatState::Ready)]);
    state.flashes.push(RowFlash {
        key: RowKey::Seat(SeatId::parse("s1").expect("seat")),
        at: Instant::now(),
    });
    insta::assert_snapshot!(paint(&state));
}

#[test]
fn twenty_seat_frame_is_under_the_release_budget() {
    let seats = (0..20)
        .map(|index| {
            seat(
                &format!("s{index}"),
                &format!("seat{index}"),
                SeatState::Ready,
            )
        })
        .collect();
    let state = sized(120, 40, seats);
    let start = Instant::now();
    for _ in 0..10 {
        let _painted = paint(&state);
    }
    let each = start.elapsed() / 10;
    let budget = if cfg!(debug_assertions) {
        std::time::Duration::from_millis(50)
    } else {
        std::time::Duration::from_millis(5)
    };
    assert!(each < budget, "frame took {each:?}, budget {budget:?}");
}

#[test]
fn row_flash_80x24() {
    let mut state = sized(80, 24, vec![seat("s1", "alpha", SeatState::Ready)]);
    state.flashes.push(RowFlash {
        key: RowKey::Seat(SeatId::parse("s1").expect("seat")),
        at: Instant::now(),
    });
    insta::assert_snapshot!(paint(&state));
}
