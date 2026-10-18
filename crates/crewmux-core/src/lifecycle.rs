// Derived from openrig packages/daemon/src/domain/seat-lifecycle-service.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Pure up/down planning. I/O stays in the daemon.

use crewmux_types::{LoginKind, ReadinessCode, RigId, SeatId, SessionId};

use crate::clock::Timestamp;
use crate::error::Error;
use crate::event::DomainEvent;
use crate::model::{Rig, Seat, SeatState};
use crewmux_spec::RigSpec;

/// Ids the caller mints so tests stay deterministic.
pub struct PlanIds {
    /// Rig id.
    pub rig: RigId,
    /// One seat id per seat, in spec order.
    pub seats: Vec<SeatId>,
    /// One session id per seat, in spec order.
    pub sessions: Vec<SessionId>,
}

/// Result of [`LifecycleService::plan_up`].
pub struct UpPlan {
    /// The rig row to insert.
    pub rig: Rig,
    /// Seat rows in spec order, all [`SeatState::Planned`].
    pub seats: Vec<Seat>,
    /// `RigCreated` only. Launch events come from [`Self::apply_launch_result`].
    pub events: Vec<DomainEvent>,
}

/// What the multiplexer reported for one seat.
pub enum LaunchApply {
    /// The session exists.
    Started {
        /// Session id from the plan.
        session: SessionId,
        /// tmux session name.
        tmux_session: String,
        /// Pane id, when known.
        pane_id: Option<String>,
    },
    /// The seat did not get a session.
    Failed {
        /// Attention code.
        code: ReadinessCode,
        /// Human-readable reason.
        reason: String,
    },
}

/// Pure transitions for one up or down.
#[derive(Debug, Default)]
pub struct LifecycleService;

impl LifecycleService {
    /// Plan a rig and its seats. Does not launch anything.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Invalid`] when the id list length does not match the spec.
    pub fn plan_up(spec: &RigSpec, ids: &PlanIds, now: Timestamp) -> Result<UpPlan, Error> {
        if ids.sessions.len() != spec.seats.len() || ids.seats.len() != spec.seats.len() {
            return Err(Error::Invalid(
                "plan ids must include one seat id and one session id per seat".to_owned(),
            ));
        }
        let rig = Rig {
            id: ids.rig,
            name: spec.name.clone(),
            created_at: now,
        };
        let mut seats = Vec::with_capacity(spec.seats.len());
        for (seat, id) in spec.seats.iter().zip(&ids.seats) {
            seats.push(Seat {
                id: *id,
                rig_id: rig.id,
                name: seat.name.clone(),
                runtime: seat.runtime,
                cwd: seat.cwd.clone(),
                model: seat.model.clone(),
                state: SeatState::Planned,
                reason: None,
                tmux_session: None,
                pane_id: None,
            });
        }
        let events = vec![DomainEvent::RigCreated {
            rig: rig.id,
            name: rig.name.clone(),
            at: now,
        }];
        Ok(UpPlan { rig, seats, events })
    }

    /// Apply one launch result. Returns the event to append, if the launch started.
    #[must_use]
    pub fn apply_launch_result(
        seat: &mut Seat,
        result: LaunchApply,
        now: Timestamp,
    ) -> Option<DomainEvent> {
        match result {
            LaunchApply::Started {
                session,
                tmux_session,
                pane_id,
            } => {
                seat.state = SeatState::Launching;
                seat.tmux_session = Some(tmux_session);
                seat.pane_id = pane_id;
                seat.reason = None;
                Some(DomainEvent::SeatLaunched {
                    seat: seat.id,
                    session,
                    at: now,
                })
            }
            LaunchApply::Failed { code, reason } => {
                seat.state = SeatState::Attention(code);
                seat.reason = Some(reason);
                None
            }
        }
    }

    /// The composer matched.
    #[must_use]
    pub fn mark_ready(seat: &mut Seat, now: Timestamp) -> DomainEvent {
        seat.state = SeatState::Ready;
        seat.reason = None;
        DomainEvent::SeatReady {
            seat: seat.id,
            at: now,
        }
    }

    /// A gate or an unrecognised screen.
    pub fn mark_attention(seat: &mut Seat, code: ReadinessCode, reason: Option<String>) {
        seat.state = SeatState::Attention(code);
        seat.reason = reason;
    }

    /// Codex pre-flight or a login screen. The session, when one exists, is left in place.
    pub fn mark_login_required(seat: &mut Seat, kind: LoginKind, evidence: String) {
        seat.reason = Some(evidence.clone());
        seat.state = SeatState::LoginRequired { kind, evidence };
    }

    /// The pane process exited.
    ///
    /// `exit_status` is tmux `pane_dead_status`. `signal` is `pane_dead_signal`
    /// when the process was killed. `capture` is the last pane text. Any of
    /// them may be missing when the multiplexer could not read it.
    #[must_use]
    pub fn mark_dead(
        seat: &mut Seat,
        now: Timestamp,
        exit_status: Option<i32>,
        signal: Option<String>,
        capture: Option<String>,
    ) -> DomainEvent {
        seat.state = SeatState::Dead;
        DomainEvent::SeatDied {
            seat: seat.id,
            at: now,
            exit_status,
            signal,
            capture,
        }
    }

    /// Mark every seat that is not already stopped. Dead seats are stopped too.
    #[must_use]
    pub fn plan_down(seats: &mut [Seat], now: Timestamp) -> Vec<DomainEvent> {
        let mut events = Vec::new();
        for seat in seats {
            if matches!(seat.state, SeatState::Stopped) {
                continue;
            }
            events.push(Self::mark_stopped(seat, now));
        }
        events
    }

    /// `rig down` stopped this seat.
    #[must_use]
    pub fn mark_stopped(seat: &mut Seat, now: Timestamp) -> DomainEvent {
        seat.state = SeatState::Stopped;
        DomainEvent::SeatStopped {
            seat: seat.id,
            at: now,
        }
    }
}
