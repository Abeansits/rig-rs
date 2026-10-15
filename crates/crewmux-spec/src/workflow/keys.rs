//! Closed keysets from `workflow-spec-cache.ts`.

pub(crate) const TOP: &[&str] = &[
    "id",
    "version",
    "objective",
    "target",
    "entry",
    "roles",
    "steps",
    "invariants",
    "closure",
    "loop_guards",
    "exception_routing",
    "coordination_terminal_turn_rule",
    "context_refs",
];

pub(crate) const STEP: &[&str] = &[
    "id",
    "actor_role",
    "objective",
    "allowed_exits",
    "re_present_after_seconds",
    "re_present_max_seconds",
    "next_hop",
    "harness",
    "host",
    "gate",
    "depends_on",
    "acceptance",
];

pub(crate) const ROLE: &[&str] = &["skill_refs", "preferred_targets"];
pub(crate) const NEXT_HOP: &[&str] = &["mode", "suggested_roles", "on"];
pub(crate) const GATE: &[&str] = &["target", "summary", "evidence_ref"];
pub(crate) const ACCEPTANCE: &[&str] = &["candidate", "verdicts", "evidence_ref"];
pub(crate) const TARGET: &[&str] = &["rig"];
pub(crate) const ENTRY: &[&str] = &["role"];
pub(crate) const INVARIANTS: &[&str] = &[
    "continuation_required",
    "allowed_exits",
    "preserve_lineage",
    "closure_required",
];
pub(crate) const CLOSURE: &[&str] = &["success", "degraded", "failed"];
pub(crate) const LOOP: &[&str] = &["max_hops", "spawn_budget"];
pub(crate) const ROUTING: &[&str] = &["default", "orchestrator_role", "classes"];
