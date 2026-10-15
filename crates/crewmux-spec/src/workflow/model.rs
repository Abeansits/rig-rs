//! Parsed workflow spec. Unknown keys never reach this type; the parser rejects them.

use std::collections::BTreeMap;

/// A workflow document after a successful parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowSpec {
    /// `workflow.id`.
    pub id: String,
    /// Integer `workflow.version` when the YAML value is an integer.
    pub version: u32,
    /// TypeScript `String(version)`, which is what the wire and the cache store.
    pub version_label: String,
    /// `workflow.objective`, when present.
    pub objective: Option<String>,
    /// `workflow.target.rig`, when present.
    pub target_rig: Option<String>,
    /// `workflow.entry.role`, when present.
    pub entry_role: Option<String>,
    /// `workflow.coordination_terminal_turn_rule`, when present.
    pub coordination_terminal_turn_rule: Option<String>,
    /// Declared roles, keyed by name.
    pub roles: BTreeMap<String, RoleSpec>,
    /// Role names in source order. The advisory that names one role uses this order.
    pub role_order: Vec<String>,
    /// Steps in declaration order. `steps[0]` is the entry step.
    pub steps: Vec<StepSpec>,
    /// `workflow.invariants`, when present.
    pub invariants: Option<Invariants>,
    /// Display-only closure messages.
    pub closure: Option<ClosureText>,
    /// Loop guards. A null `max_hops` is absent.
    pub loop_guards: Option<LoopGuards>,
    /// Exception dial, when present.
    pub exception_routing: Option<ExceptionRouting>,
    /// Context addresses, when present.
    pub context_refs: Option<Vec<String>>,
    /// Path the parser was given. It is quoted in error text and stored on the cache row.
    pub source_path: String,
}

/// One role.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RoleSpec {
    /// Documentation-only skill names.
    pub skill_refs: Vec<String>,
    /// Owner candidates, in declaration order. Not liveness-filtered.
    pub preferred_targets: Vec<String>,
}

/// One step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepSpec {
    /// Stable id. Empty when the author omitted it; the validator reports that.
    pub id: String,
    /// Declared actor. Empty when omitted.
    pub actor_role: String,
    /// Step objective, when present.
    pub objective: Option<String>,
    /// Author-declared exits. `None` means the step does not restrict exits.
    pub allowed_exits: Option<Vec<String>>,
    /// One-shot wait reminder delay, in seconds.
    pub re_present_after_seconds: Option<i64>,
    /// Upper bound for the reminder, in seconds.
    pub re_present_max_seconds: Option<i64>,
    /// Routing hint.
    pub next_hop: Option<NextHop>,
    /// Agent harness pin (`claude-code` or `codex`).
    pub harness: Option<String>,
    /// Host pin. `local` is always legal.
    pub host: Option<String>,
    /// Structured gate.
    pub gate: Option<GateSpec>,
    /// Prerequisite step ids.
    pub depends_on: Option<Vec<String>>,
    /// Acceptance payload required to leave the step.
    pub acceptance: Option<Acceptance>,
}

/// `next_hop` routing.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct NextHop {
    /// `require` or `forbid`. `prefer` is rejected at parse.
    pub mode: Option<String>,
    /// Roles tried in order before declaration-order fallback.
    pub suggested_roles: Vec<String>,
    /// Branch edges. Keys are the closed exit enum.
    pub on: Vec<(String, String)>,
}

impl NextHop {
    /// Mapped step id for `exit`, if the author wrote that key.
    #[must_use]
    pub fn mapped(&self, exit: &str) -> Option<&str> {
        self.on
            .iter()
            .find(|(key, _)| key == exit)
            .map(|(_, target)| target.as_str())
    }
}

/// Human or handler gate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateSpec {
    /// Human seat session or declared role name.
    pub target: String,
    /// Plain-language ask. Required for a human target.
    pub summary: Option<String>,
    /// Durable evidence pointer. Required for a human target.
    pub evidence_ref: Option<String>,
}

/// Acceptance gate payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Acceptance {
    /// Expected candidate.
    pub candidate: String,
    /// Allowed verdicts.
    pub verdicts: Vec<String>,
    /// Expected evidence pointer.
    pub evidence_ref: String,
}

/// Invariants. Several flags are recorded and not enforced.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Invariants {
    /// Recorded, not enforced.
    pub continuation_required: Option<bool>,
    /// Exits a step may use, when set.
    pub allowed_exits: Option<Vec<String>>,
    /// Recorded, not enforced. Lineage is always preserved.
    pub preserve_lineage: Option<bool>,
    /// Recorded, not enforced. Closure is always required.
    pub closure_required: Option<bool>,
}

/// Display-only closure messages.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ClosureText {
    /// Success message.
    pub success: Option<String>,
    /// Degraded message.
    pub degraded: Option<String>,
    /// Failed message.
    pub failed: Option<String>,
}

/// Hop and spawn budgets.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct LoopGuards {
    /// Enforced hop ceiling. `None` does not sanction a cycle.
    pub max_hops: Option<i64>,
    /// Recorded, not enforced.
    pub spawn_budget: Option<i64>,
}

/// Exception dial declared on the spec.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ExceptionRouting {
    /// Workflow default position.
    pub default: Option<String>,
    /// Role resolved for the orchestrator position.
    pub orchestrator_role: Option<String>,
    /// Per-class positions. `human_gate_trip` is not configurable.
    pub classes: BTreeMap<String, String>,
}

/// Closed exit names, in TypeScript order.
pub const EXIT_KINDS: [&str; 4] = ["handoff", "waiting", "done", "failed"];

/// Agent harnesses a step may pin.
pub const AGENT_HARNESSES: [&str; 2] = ["claude-code", "codex"];
