//! Insta goldens at 120×40 and 80×24.

use std::time::Instant;

use crewmux_api::{
    HealthSummary, PodName, ReadinessCode, RigId, RigName, RuntimeKind, SeatId, SeatName,
    SeatState, SeatView, SessionId, Snapshot,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::msg::{KeyEvent, KeyKind, Msg};
use crate::rows;
use crate::state::{RowFlash, RowKey, Section, Size, State};
use crate::update::update;
use crate::view::view;

fn seat(id: &str, name: &str, state: SeatState) -> SeatView {
    seat_runtime(id, name, state, RuntimeKind::Stub)
}

fn seat_runtime(id: &str, name: &str, state: SeatState, runtime: RuntimeKind) -> SeatView {
    SeatView {
        id: SeatId::parse(id).expect("seat"),
        name: SeatName::parse(name).expect("name"),
        rig_id: RigId::parse("rig-1").expect("rig"),
        rig_name: RigName::parse("demo").expect("rig name"),
        pod: PodName::parse("pod").expect("pod"),
        state,
        runtime,
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
    let state = State::new(Size::new(cols, rows), Instant::now());
    let (state, _) = update(
        state,
        Msg::Hydrated(Ok(Snapshot {
            seats,
            health: None,
            activity: vec![],
        })),
    );
    state
}

fn attention(cols: u16, rows: u16) -> State {
    let state = sized(
        cols,
        rows,
        vec![seat(
            "s1",
            "alpha",
            SeatState::Attention(ReadinessCode::LoginRequired),
        )],
    );
    let (state, _) = update(
        state,
        Msg::Key(KeyEvent {
            kind: KeyKind::Char('3'),
        }),
    );
    state
}

fn flashed(cols: u16, rows: u16) -> State {
    let mut state = sized(cols, rows, vec![seat("s1", "alpha", SeatState::Ready)]);
    state.flashes.push(RowFlash {
        key: RowKey::Seat(SeatId::parse("s1").expect("seat")),
        at: state.now,
    });
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
    insta::assert_snapshot!(paint(&attention(120, 40)));
}

#[test]
fn attention_login_required_80x24() {
    insta::assert_snapshot!(paint(&attention(80, 24)));
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
    insta::assert_snapshot!(paint(&flashed(120, 40)));
}

#[test]
fn row_flash_80x24() {
    insta::assert_snapshot!(paint(&flashed(80, 24)));
}

#[test]
fn attention_dead_two_seats_and_flash_are_distinct() {
    let attention = paint(&attention(120, 40));
    let dead = paint(&sized(120, 40, vec![seat("s1", "alpha", SeatState::Dead)]));
    let two = paint(&sized(
        120,
        40,
        vec![
            seat("s1", "alpha", SeatState::Ready),
            seat("s2", "beta", SeatState::Ready),
        ],
    ));
    let flashed = paint(&flashed(120, 40));
    let screens = [&attention, &dead, &two, &flashed];
    for (index, left) in screens.iter().enumerate() {
        for right in screens.iter().skip(index.saturating_add(1)) {
            assert_ne!(left, right);
        }
    }
    assert!(attention.contains("login_required"), "{attention}");
    assert!(dead.contains("dead"), "{dead}");
    assert!(two.contains("beta"), "{two}");
    assert!(flashed.contains("alpha"), "{flashed}");
    assert!(!flashed.contains("login_required"));
}

#[test]
fn explorer_rows_omit_runtime_words_and_mark_glyphs() {
    let seats = vec![
        seat_runtime("s1", "alpha", SeatState::Ready, RuntimeKind::ClaudeCode),
        seat_runtime("s2", "beta", SeatState::Ready, RuntimeKind::Codex),
        seat_runtime("s3", "gamma", SeatState::Ready, RuntimeKind::Terminal),
    ];
    let painted = paint(&sized(140, 40, seats));
    let lower = painted.to_ascii_lowercase();
    for word in ["claude", "codex", "terminal", "tty"] {
        assert!(!lower.contains(word), "{word} leaked into {painted}");
    }
    assert!(!painted.contains(">_"), "{painted}");
}

#[test]
fn health_unavailable_and_empty_are_distinct() {
    let mut state = sized(80, 24, vec![]);
    state.section = Section::Health;
    let unavailable = rows::content_rows(&state);
    state.snapshot.health = Some(HealthSummary { records: vec![] });
    let empty = rows::content_rows(&state);
    assert_ne!(unavailable, empty);
    assert_eq!(
        unavailable.first().map(|row| row.text.as_str()),
        Some("health: unavailable")
    );
    assert_eq!(
        empty.first().map(|row| row.text.as_str()),
        Some("health: no findings")
    );
}

#[test]
fn twenty_seat_frame_is_under_five_milliseconds_in_release() {
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
    assert!(
        each < budget,
        "frame took {each:?}, budget {budget:?} (5ms is the release gate)"
    );
}
