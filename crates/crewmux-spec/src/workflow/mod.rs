//! Workflow spec YAML. The only YAML parser in this crate is [`crate::yaml`].

mod assemble;
mod error;
mod keys;
mod model;
mod order;
mod parse;
mod steps;
mod value;

pub use error::WorkflowParseError;
pub use model::{
    AGENT_HARNESSES, Acceptance, ClosureText, EXIT_KINDS, ExceptionRouting, GateSpec, Invariants,
    LoopGuards, NextHop, RoleSpec, StepSpec, WorkflowSpec,
};
pub use parse::parse_workflow;
