//! Workflow engine: spec validation, the transactional projector, and boot sweep.
//!
//! The runtime is pure. It clones a queue book and an [`Engine`], mutates the
//! clones, and returns them. Callers publish the pair once. SQLite lives in
//! `crewmux-store`. This crate does not start a runtime or spawn a process.
//! Context packs live in [`context_packs`].

pub mod context_packs;

mod boot;
mod builtin;
mod deadline;
mod dial;
mod error;
mod exit;
mod frontier;
mod inspect;
mod keepalive;
mod model;
mod packet;
mod project;
mod role;
mod route;
mod runtime;
mod time;
mod validate;
mod wire;
mod world;

pub use boot::{Sweep, boot_sweep};
pub use builtin::install as install_builtins;
pub use deadline::{
    MAX_HOPS_BASELINE_V1, STUCK_THRESHOLD_SECONDS, Verdict, evaluate_deadline, exceeds_max_hops,
};
pub use dial::{DialChoice, OPERATOR_SEAT, route_exception};
pub use error::Error;
pub use exit::Exit;
pub use frontier::frontier_block;
pub use inspect::{Inspected, inspect};
pub use model::{CachedSpec, Decision, Engine, Instance, Keepalive, Phase, Status, Trail};
pub use project::ProjectRequest;
pub use project::{ProjectCtx, Projection, project_step};
pub use role::{Disqualified, RolePick, SeatCandidate, select_role_seat};
pub use route::resolve_next_step;
pub use runtime::{
    ContinueView, Instantiate, InstantiateRequest, Runtime, deadline_for, guidance_lines,
    parse_exit,
};
pub use validate::{Issue, Severity, Validation, validate};
pub use wire::{
    instance_json, instantiate_spec_json, project_json, spec_row_json, trail_json, validation_json,
};
pub use world::World;
