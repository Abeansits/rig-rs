//! Pure domain logic: rigs, seats, sessions, lifecycle, and events.
//!
//! No tokio, no rusqlite, and no process spawning. The daemon applies these
//! plans inside a store transaction.

mod clock;
mod error;
mod event;
mod lifecycle;
mod model;
mod port;

pub use clock::{Clock, ManualClock, SystemClock, Timestamp};
pub use error::Error;
pub use event::{DomainEvent, Seq};
pub use lifecycle::{LaunchApply, LifecycleService, PlanIds, UpPlan};
pub use model::{Rig, Seat, SeatState};
pub use port::{EventLog, RigRepo, SeatRepo, SessionRepo};
