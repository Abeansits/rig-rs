//! Pure up/down plans, including attention, death, and a partial launch.

use std::time::Duration;

use crewmux_core::{
    Clock, DomainEvent, LaunchApply, LifecycleService, ManualClock, PlanIds, SeatState, Timestamp,
};
use crewmux_spec::parse;
use crewmux_types::{ReadinessCode, RigId, SeatId, SessionId};

const SPEC: &str = "\
name: pair
seats:
  - name: one
    runtime: stub
  - name: two
    runtime: stub
";

fn ids() -> PlanIds {
    PlanIds {
        rig: RigId::new(),
        seats: vec![SeatId::new(), SeatId::new()],
        sessions: vec![SessionId::new(), SessionId::new()],
    }
}

#[test]
fn plan_up_emits_rig_created_and_planned_seats() {
    let spec = parse(SPEC).expect("spec");
    let minted = ids();
    let now = Timestamp::from_millis(10);
    let plan = LifecycleService::plan_up(&spec, &minted, now).expect("plan");
    assert_eq!(plan.seats.len(), 2);
    assert!(
        plan.seats
            .iter()
            .all(|seat| seat.state == SeatState::Planned)
    );
    assert!(matches!(
        plan.events.first(),
        Some(DomainEvent::RigCreated { .. })
    ));
}

#[test]
fn launch_then_ready_is_causal() {
    let spec = parse(SPEC).expect("spec");
    let minted = ids();
    let now = Timestamp::from_millis(1);
    let mut plan = LifecycleService::plan_up(&spec, &minted, now).expect("plan");
    let seat = plan.seats.first_mut().expect("seat");
    let launched = LifecycleService::apply_launch_result(
        seat,
        LaunchApply::Started {
            session: minted.sessions[0],
            tmux_session: "one__pair".to_owned(),
            pane_id: Some("%1".to_owned()),
        },
        now,
    );
    assert!(matches!(launched, Some(DomainEvent::SeatLaunched { .. })));
    assert_eq!(seat.state, SeatState::Launching);
    let ready = LifecycleService::mark_ready(seat, now);
    assert!(matches!(ready, DomainEvent::SeatReady { .. }));
    assert_eq!(seat.state.as_str(), "ready");
}

#[test]
fn gate_is_attention_and_not_ready() {
    let spec = parse(SPEC).expect("spec");
    let minted = ids();
    let mut plan =
        LifecycleService::plan_up(&spec, &minted, Timestamp::from_millis(1)).expect("plan");
    let seat = plan.seats.first_mut().expect("seat");
    drop(LifecycleService::apply_launch_result(
        seat,
        LaunchApply::Started {
            session: minted.sessions[0],
            tmux_session: "one__pair".to_owned(),
            pane_id: Some("%1".to_owned()),
        },
        Timestamp::from_millis(1),
    ));
    LifecycleService::mark_attention(seat, ReadinessCode::LoginRequired, Some("login".to_owned()));
    assert_eq!(seat.state.as_str(), "attention");
    assert_eq!(seat.state.code(), Some(&ReadinessCode::LoginRequired));
    assert!(
        seat.state
            .code()
            .is_some_and(ReadinessCode::requires_attention)
    );
    assert_ne!(seat.state, SeatState::Ready);
}

#[test]
fn death_and_down_emit_their_events() {
    let spec = parse(SPEC).expect("spec");
    let minted = ids();
    let now = Timestamp::from_millis(5);
    let mut plan = LifecycleService::plan_up(&spec, &minted, now).expect("plan");
    let first = plan.seats.first_mut().expect("seat");
    let died = LifecycleService::mark_dead(first, now);
    assert!(matches!(died, DomainEvent::SeatDied { .. }));
    assert_eq!(first.state, SeatState::Dead);
    let second = plan.seats.get_mut(1).expect("seat");
    let stopped = LifecycleService::mark_stopped(second, now);
    assert!(matches!(stopped, DomainEvent::SeatStopped { .. }));
    assert_eq!(second.state.as_str(), "stopped");
}

#[test]
fn partial_failure_leaves_the_other_seat_launchable() {
    let spec = parse(SPEC).expect("spec");
    let minted = ids();
    let now = Timestamp::from_millis(1);
    let mut plan = LifecycleService::plan_up(&spec, &minted, now).expect("plan");
    let bad = plan.seats.first_mut().expect("seat");
    let failed = LifecycleService::apply_launch_result(
        bad,
        LaunchApply::Failed {
            code: ReadinessCode::Other("invalid_cwd".to_owned()),
            reason: "cwd does not exist".to_owned(),
        },
        now,
    );
    assert!(failed.is_none());
    assert_eq!(bad.state.as_str(), "attention");
    let good = plan.seats.get_mut(1).expect("seat");
    let started = LifecycleService::apply_launch_result(
        good,
        LaunchApply::Started {
            session: minted.sessions[1],
            tmux_session: "two__pair".to_owned(),
            pane_id: Some("%2".to_owned()),
        },
        now,
    );
    assert!(started.is_some());
    assert_eq!(good.state, SeatState::Launching);
}

#[test]
fn plan_down_stops_live_seats() {
    let spec = parse(SPEC).expect("spec");
    let minted = ids();
    let now = Timestamp::from_millis(9);
    let mut plan = LifecycleService::plan_up(&spec, &minted, now).expect("plan");
    let events = LifecycleService::plan_down(&mut plan.seats, now);
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|event| event.kind() == "seat_stopped"));
    assert!(
        plan.seats
            .iter()
            .all(|seat| seat.state == SeatState::Stopped)
    );
    let again = LifecycleService::plan_down(&mut plan.seats, now);
    assert_eq!(again.len(), 0);
}

#[test]
fn manual_clock_advances_without_sleeping() {
    let clock = ManualClock::new(Timestamp::from_millis(0));
    clock.advance(Duration::from_secs(15));
    assert_eq!(clock.now().millis(), 15_000);
}
