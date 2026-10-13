// Derived from openrig packages/daemon/src/domain/workflow-exception-router.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Exception dial. `human_gate_trip` is intrinsically human-only.
//!
//! The human seat is the operator surface (`human@kernel`): TUI attention and
//! the CLI inbox. It is not the Slack gateway registry. See PE-021.

use crewmux_spec::workflow::WorkflowSpec;

use crate::world::World;

/// Operator-surface human seat. PE-021.
pub const OPERATOR_SEAT: &str = "human@kernel";

/// Where an exception packet goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialChoice {
    /// Destination session.
    pub session: String,
    /// `human-gate` or `mode2`.
    pub tier: &'static str,
    /// Position that won.
    pub position: &'static str,
}

/// Route `class` (`unmapped_failed`, `stuck_overdue`, or `human_gate_trip`).
#[must_use]
pub fn route_exception(spec: &WorkflowSpec, class: &str, world: &World) -> DialChoice {
    if class == "human_gate_trip" {
        return human();
    }
    let position = spec
        .exception_routing
        .as_ref()
        .and_then(|routing| routing.classes.get(class).cloned())
        .or_else(|| {
            spec.exception_routing
                .as_ref()
                .and_then(|routing| routing.default.clone())
        })
        .or_else(|| world.host_default.clone())
        .unwrap_or_else(|| "orchestrator".to_owned());
    if position == "human_only" {
        return human();
    }
    orchestrator(spec, world)
}

fn human() -> DialChoice {
    DialChoice {
        session: OPERATOR_SEAT.to_owned(),
        tier: "human-gate",
        position: "human_only",
    }
}

fn orchestrator(spec: &WorkflowSpec, world: &World) -> DialChoice {
    let role = spec
        .exception_routing
        .as_ref()
        .and_then(|routing| routing.orchestrator_role.clone());
    let target = role.as_ref().and_then(|role| {
        spec.roles
            .get(role)
            .and_then(|spec| spec.preferred_targets.first().cloned())
    });
    if let Some(session) = target {
        return DialChoice {
            session,
            tier: "mode2",
            position: "orchestrator",
        };
    }
    let _ = world;
    DialChoice {
        session: OPERATOR_SEAT.to_owned(),
        tier: "human-gate",
        position: "fallback",
    }
}
