// Derived from openrig packages/daemon/src/domain/workflow-validator.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Spec validation. Issue codes, fields, and messages match TypeScript.

use std::collections::{BTreeMap, BTreeSet};

use crewmux_messaging::is_human_seat;
use crewmux_spec::workflow::{StepSpec, WorkflowSpec};
use serde::Serialize;

use crate::route::resolve_next_step;

/// `error` blocks. `warning` does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Blocks instantiate.
    Error,
    /// Recorded. Exit stays 0.
    Warning,
}

/// One issue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Issue {
    /// Stable code.
    pub code: &'static str,
    /// What, why, and what to do.
    pub message: String,
    /// Field path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// Severity.
    pub severity: Severity,
}

/// Validation report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Validation {
    /// True when there is no error.
    pub ok: bool,
    /// Issues in discovery order.
    pub issues: Vec<Issue>,
    /// Summary.
    pub summary: Summary,
}

/// Summary block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    /// Spec id.
    pub workflow_id: String,
    /// Version label.
    pub workflow_version: String,
    /// `target.rig`.
    pub target_rig: Option<String>,
    /// `entry.role`.
    pub entry_role: Option<String>,
    /// Step count.
    pub step_count: usize,
}

/// Optional probes. Absent skips the check.
pub struct Probes<'a> {
    /// Seat liveness. A warning when no preferred target is live.
    pub liveness: Option<&'a dyn Fn(&str) -> bool>,
    /// Host registry. `local` is always legal.
    pub hosts: Option<&'a dyn Fn(&str) -> HostProbe>,
}

/// Host membership.
pub struct HostProbe {
    /// Registered.
    pub registered: bool,
    /// Known ids.
    pub ids: Vec<String>,
}

/// Validate `spec`.
#[must_use]
pub fn validate(spec: &WorkflowSpec, probes: &Probes<'_>) -> Validation {
    let mut issues = Vec::new();
    let roles: BTreeSet<&str> = spec.roles.keys().map(String::as_str).collect();
    entry_issues(spec, &roles, &mut issues);
    step_issues(spec, &roles, &mut issues);
    hop_issues(spec, &roles, &mut issues);
    branch_issues(spec, &mut issues);
    finish_issues(spec, &mut issues);
    if spec.steps.iter().all(|step| !step.id.is_empty()) && !spec.steps.is_empty() {
        graph_issues(spec, &mut issues);
    }
    host_issues(spec, probes.hosts, &mut issues);
    gate_issues(spec, &roles, &mut issues);
    advisory_issues(spec, &mut issues);
    liveness_issues(spec, probes.liveness, &mut issues);
    let ok = !issues.iter().any(|issue| issue.severity == Severity::Error);
    Validation {
        ok,
        issues,
        summary: Summary {
            workflow_id: spec.id.clone(),
            workflow_version: spec.version_label.clone(),
            target_rig: spec.target_rig.clone(),
            entry_role: spec.entry_role.clone(),
            step_count: spec.steps.len(),
        },
    }
}

fn push(
    issues: &mut Vec<Issue>,
    code: &'static str,
    message: impl Into<String>,
    field: impl Into<String>,
    severity: Severity,
) {
    issues.push(Issue {
        code,
        message: message.into(),
        field: Some(field.into()),
        severity,
    });
}

fn entry_issues(spec: &WorkflowSpec, roles: &BTreeSet<&str>, issues: &mut Vec<Issue>) {
    let Some(role) = spec.entry_role.as_deref() else {
        return;
    };
    if !roles.contains(role) {
        push(
            issues,
            "entry_role_not_declared",
            format!(
                "entry role \"{role}\" is not declared in workflow.roles. Add the role to workflow.roles or change the entry to a declared role."
            ),
            "workflow.entry.role",
            Severity::Error,
        );
    }
    let Some(first) = spec.steps.first() else {
        return;
    };
    if first.actor_role.is_empty() || role == first.actor_role {
        return;
    }
    push(
        issues,
        "entry_role_mismatch",
        format!(
            "workflow.entry.role is \"{role}\" but the authoritative entry is steps[0] (\"{}\", actor_role \"{}\") — the runtime instantiates from steps[0], so a differing entry.role would lie on every surface that reports it. Reorder steps so the intended entry step is first, or fix/remove entry.role.",
            first.id, first.actor_role
        ),
        "workflow.entry.role",
        Severity::Error,
    );
}

fn step_issues(spec: &WorkflowSpec, roles: &BTreeSet<&str>, issues: &mut Vec<Issue>) {
    let mut seen = BTreeSet::new();
    let invariant = spec
        .invariants
        .as_ref()
        .and_then(|invariants| invariants.allowed_exits.as_ref());
    for (index, step) in spec.steps.iter().enumerate() {
        let base = format!("workflow.steps[{index}]");
        if step.id.is_empty() {
            push(issues, "step_id_missing", format!("step at {base} is missing an id. Every step must have a stable id used to reference it from next_hop hints and step trails."), format!("{base}.id"), Severity::Error);
            continue;
        }
        if !seen.insert(step.id.as_str()) {
            push(issues, "step_id_duplicate", format!("duplicate step id \"{}\" at {base}. Step ids must be unique within a workflow.", step.id), format!("{base}.id"), Severity::Error);
        }
        actor_issue(step, roles, &base, issues);
        exit_issues(step, invariant, &base, issues);
        depend_issues(spec, step, &base, issues);
    }
}

fn actor_issue(step: &StepSpec, roles: &BTreeSet<&str>, base: &str, issues: &mut Vec<Issue>) {
    if step.actor_role.is_empty() {
        push(issues, "step_actor_role_missing", format!("step \"{}\" is missing actor_role. Declare which role drives this step so the projector can derive the next-step owner.", step.id), format!("{base}.actor_role"), Severity::Error);
    } else if !roles.contains(step.actor_role.as_str()) {
        push(issues, "step_actor_role_not_declared", format!("step \"{}\" references role \"{}\" which is not declared in workflow.roles. Add the role to workflow.roles or change the step.", step.id, step.actor_role), format!("{base}.actor_role"), Severity::Error);
    }
}

fn exit_issues(
    step: &StepSpec,
    invariant: Option<&Vec<String>>,
    base: &str,
    issues: &mut Vec<Issue>,
) {
    if let (Some(allowed), Some(invariant)) = (step.allowed_exits.as_ref(), invariant) {
        for exit in allowed {
            if !invariant.iter().any(|item| item == exit) {
                push(issues, "step_exit_not_allowed", format!("step \"{}\" allows exit \"{exit}\" which is not in workflow.invariants.allowed_exits. Either remove the exit from the step or extend the invariant.", step.id), format!("{base}.allowed_exits"), Severity::Error);
            }
        }
    }
    if step.re_present_after_seconds.is_some()
        && step.allowed_exits.as_ref().is_some_and(|exits| !exits.is_empty())
        && step.allowed_exits.as_ref().is_some_and(|exits| !exits.iter().any(|exit| exit == "waiting"))
    {
        push(issues, "waiting_re_presentation_unreachable", format!("step \"{}\" declares re_present_after_seconds but does not allow the waiting exit. Add \"waiting\" to allowed_exits or remove the inert deadline.", step.id), format!("{base}.re_present_after_seconds"), Severity::Error);
    }
    if step.re_present_after_seconds.is_some()
        && step.next_hop.as_ref().is_some_and(|hop| hop.mapped("waiting").is_some())
    {
        let target = step.next_hop.as_ref().and_then(|hop| hop.mapped("waiting")).unwrap_or("");
        push(issues, "waiting_re_presentation_unreachable", format!("step \"{}\" maps the waiting exit to \"{target}\", so waiting routes immediately and never parks. Remove re_present_after_seconds or remove the waiting branch.", step.id), format!("{base}.re_present_after_seconds"), Severity::Error);
    }
}

fn depend_issues(spec: &WorkflowSpec, step: &StepSpec, base: &str, issues: &mut Vec<Issue>) {
    for dependency in step.depends_on.iter().flatten() {
        if dependency == &step.id {
            push(issues, "dependency_self_reference", format!("step \"{}\" cannot depend on itself.", step.id), format!("{base}.depends_on"), Severity::Error);
        } else if !spec.steps.iter().any(|candidate| candidate.id == *dependency) {
            push(issues, "dependency_step_not_found", format!("step \"{}\" depends on missing step \"{dependency}\". Fix the id or remove the dead prerequisite.", step.id), format!("{base}.depends_on"), Severity::Error);
        }
    }
}

fn hop_issues(spec: &WorkflowSpec, roles: &BTreeSet<&str>, issues: &mut Vec<Issue>) {
    for (index, step) in spec.steps.iter().enumerate() {
        let field = format!("workflow.steps[{index}].next_hop.suggested_roles");
        let Some(hop) = step.next_hop.as_ref() else {
            continue;
        };
        for role in &hop.suggested_roles {
            if !roles.contains(role.as_str()) {
                push(issues, "next_hop_role_not_declared", format!("step \"{}\" suggests next-hop role \"{role}\" which is not declared in workflow.roles. Declare the role or fix the spelling — the edge can never route.", step.id), field.clone(), Severity::Error);
            } else if !spec.steps.iter().any(|candidate| candidate.actor_role == *role) {
                push(issues, "next_hop_role_has_no_step", format!("step \"{}\" suggests next-hop role \"{role}\" but no step declares actor_role \"{role}\" — resolveNextStep matches suggestions against step actor_roles, so this edge can never route. Add a step for the role or fix the suggestion.", step.id), field.clone(), Severity::Error);
            }
        }
    }
}

fn branch_issues(spec: &WorkflowSpec, issues: &mut Vec<Issue>) {
    for (index, step) in spec.steps.iter().enumerate() {
        let Some(hop) = step.next_hop.as_ref() else {
            continue;
        };
        for (exit, target) in &hop.on {
            if !target.is_empty() && !spec.steps.iter().any(|candidate| candidate.id == *target) {
                push(issues, "branch_target_not_found", format!("step \"{}\" branches exit \"{exit}\" to step \"{target}\" which does not exist. Fix the step id or remove the branch — the edge can never route.", step.id), format!("workflow.steps[{index}].next_hop.on.{exit}"), Severity::Error);
            }
        }
    }
}

fn finish_issues(spec: &WorkflowSpec, issues: &mut Vec<Issue>) {
    let graph = spec.steps.iter().any(|step| step.depends_on.is_some());
    for (index, step) in spec.steps.iter().enumerate() {
        let Some(exits) = step.allowed_exits.as_ref().filter(|exits| !exits.is_empty()) else {
            continue;
        };
        if step.id.is_empty() || exits.iter().any(|exit| exit == "done") {
            continue;
        }
        if exits.iter().any(|exit| {
            step.next_hop.as_ref().is_some_and(|hop| hop.mapped(exit).is_some())
        }) {
            continue;
        }
        if exits.iter().any(|exit| exit == "handoff")
            && (graph || resolve_next_step(spec, step, Some("handoff")).is_some())
        {
            continue;
        }
        let handoff = if exits.iter().any(|exit| exit == "handoff") {
            ", and handoff has no next step to route to"
        } else {
            ""
        };
        let rendered = serde_json::to_string(exits).unwrap_or_else(|_| "[]".to_owned());
        push(issues, "step_cannot_finish", format!("step \"{}\" allows exits {rendered}, but none of them can finish it: done is not allowed{handoff}. Add \"done\" to allowed_exits, or give the step a next step.", step.id), format!("workflow.steps[{index}].allowed_exits"), Severity::Error);
    }
}

fn graph_issues(spec: &WorkflowSpec, issues: &mut Vec<Issue>) {
    let graph = spec.steps.iter().any(|step| step.depends_on.is_some());
    let successors = |step: &StepSpec| successors_of(spec, step, graph);
    let Some(entry) = spec.steps.first() else {
        return;
    };
    let reachable = reachable_from(spec, &entry.id, successors);
    if let Some(cycle) = find_cycle(spec, |step| step.depends_on.clone().unwrap_or_default(), true) {
        push(issues, "dependency_cycle", format!("the prerequisite graph cycles ({}). These steps cannot become ready; remove a cyclic depends_on edge. loop_guards.max_hops bounds routing loops, not prerequisite cycles.", cycle.join(" → ")), "workflow.steps", Severity::Error);
    } else if let Some(cycle) = find_cycle(spec, |step| successors(step), false) {
        let enforceable = spec.loop_guards.as_ref().and_then(|guards| guards.max_hops).is_some_and(|hops| hops >= 1);
        if !enforceable {
            push(issues, "cycle_without_max_hops", format!("the routing graph cycles ({}) and workflow.loop_guards.max_hops is not declared — the instance would hop unbounded. Loops (including branch-created remediation loops) are legitimate only under an enforced guard: declare loop_guards.max_hops to sanction the cycle.", cycle.join(" → ")), "workflow.loop_guards.max_hops", Severity::Error);
        }
    }
    for step in &spec.steps {
        if !step.id.is_empty() && !reachable.contains(step.id.as_str()) {
            push(issues, "step_unreachable", format!("step \"{}\" is unreachable: neither the structural routing walk from the entry step (\"{}\") nor any branch edge reaches it. Fix the next_hop edges/branches or remove the dead step.", step.id, entry.id), "workflow.steps", Severity::Error);
        }
    }
}

fn successors_of(spec: &WorkflowSpec, step: &StepSpec, graph: bool) -> Vec<String> {
    let mut out = Vec::new();
    if graph {
        for candidate in &spec.steps {
            if candidate.depends_on.as_ref().is_some_and(|deps| deps.iter().any(|id| id == &step.id)) {
                out.push(candidate.id.clone());
            }
        }
    } else if let Some(next) = resolve_next_step(spec, step, None) {
        out.push(next.id.clone());
    }
    if let Some(hop) = step.next_hop.as_ref() {
        for (_, target) in &hop.on {
            if !target.is_empty()
                && spec.steps.iter().any(|candidate| candidate.id == *target)
                && !out.iter().any(|id| id == target)
            {
                out.push(target.clone());
            }
        }
    }
    out
}

fn reachable_from(
    spec: &WorkflowSpec,
    entry: &str,
    successors: impl Fn(&StepSpec) -> Vec<String>,
) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut queue = vec![entry.to_owned()];
    while let Some(id) = queue.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(step) = spec.steps.iter().find(|step| step.id == id) else {
            continue;
        };
        queue.extend(successors(step));
    }
    seen
}

fn find_cycle(
    spec: &WorkflowSpec,
    successors: impl Fn(&StepSpec) -> Vec<String>,
    all_roots: bool,
) -> Option<Vec<String>> {
    let mut color: BTreeMap<String, bool> = BTreeMap::new();
    let mut stack = Vec::new();
    let roots: Vec<&str> = if all_roots {
        spec.steps.iter().map(|step| step.id.as_str()).collect()
    } else {
        spec.steps.first().map(|step| vec![step.id.as_str()]).unwrap_or_default()
    };
    for root in roots {
        if color.contains_key(root) {
            continue;
        }
        if let Some(cycle) = dfs(spec, root, &successors, &mut color, &mut stack) {
            return Some(cycle);
        }
    }
    None
}

fn dfs(
    spec: &WorkflowSpec,
    id: &str,
    successors: &impl Fn(&StepSpec) -> Vec<String>,
    color: &mut BTreeMap<String, bool>,
    stack: &mut Vec<String>,
) -> Option<Vec<String>> {
    color.insert(id.to_owned(), false);
    stack.push(id.to_owned());
    if let Some(step) = spec.steps.iter().find(|step| step.id == id) {
        for next in successors(step) {
            if color.get(&next) == Some(&false) {
                let start = stack.iter().position(|item| item == &next).unwrap_or(0);
                let mut cycle: Vec<String> = stack.iter().skip(start).cloned().collect();
                cycle.push(next);
                return Some(cycle);
            }
            if !color.contains_key(&next)
                && let Some(cycle) = dfs(spec, &next, successors, color, stack)
            {
                return Some(cycle);
            }
        }
    }
    stack.pop();
    color.insert(id.to_owned(), true);
    None
}

fn host_issues(
    spec: &WorkflowSpec,
    hosts: Option<&dyn Fn(&str) -> HostProbe>,
    issues: &mut Vec<Issue>,
) {
    let Some(hosts) = hosts else {
        return;
    };
    for (index, step) in spec.steps.iter().enumerate() {
        let Some(host) = step.host.as_deref() else {
            continue;
        };
        if host == "local" {
            continue;
        }
        let probe = hosts(host);
        if probe.registered {
            continue;
        }
        let listed = if probe.ids.is_empty() {
            "(none)".to_owned()
        } else {
            format!("[{}]", probe.ids.join(", "))
        };
        push(issues, "host_not_registered", format!("step \"{}\" pins host \"{host}\" which is not in the hosts registry (~/.openrig/hosts.yaml). Registered ids: {listed}. Register the host with rig host add, use \"local\", or remove the pin.", step.id), format!("workflow.steps[{index}].host"), Severity::Error);
    }
}

fn gate_issues(spec: &WorkflowSpec, roles: &BTreeSet<&str>, issues: &mut Vec<Issue>) {
    for (index, step) in spec.steps.iter().enumerate() {
        let Some(gate) = step.gate.as_ref() else {
            continue;
        };
        let base = format!("workflow.steps[{index}].gate");
        if is_human_seat(&gate.target) {
            if gate.summary.is_none() || gate.evidence_ref.is_none() {
                let missing = if gate.summary.is_none() { "summary" } else { "evidence_ref" };
                push(issues, "gate_human_fields_missing", format!("step \"{}\" gates on human seat {} but is missing {missing}. A human-routed gate item must carry a plain-language summary AND a durable evidence pointer (the shipped human-route contract) — add both.", step.id, gate.target), base, Severity::Error);
            }
        } else if !roles.contains(gate.target.as_str()) {
            push(issues, "gate_target_unresolved", format!("step \"{}\" gates on \"{}\" which is neither a human seat session (human@kernel form) nor a role declared in workflow.roles. Declare the handler role or use a human seat session.", step.id, gate.target), format!("{base}.target"), Severity::Error);
        } else if spec.roles.get(&gate.target).is_none_or(|role| role.preferred_targets.is_empty()) {
            push(issues, "gate_handler_no_targets", format!("step \"{}\" gates on handler role \"{}\" which declares no preferred_targets — the gate item will have no seat to route to when it trips. Add preferred_targets to the role before instantiating.", step.id, gate.target), format!("{base}.target"), Severity::Warning);
        }
    }
}

fn advisory_issues(spec: &WorkflowSpec, issues: &mut Vec<Issue>) {
    let v2 = |issues: &mut Vec<Issue>, key: &str, field: &str, extra: Option<&str>| {
        let tail = extra.map(|text| format!(" ({text})")).unwrap_or_default();
        push(issues, "declared_not_enforced_v1", format!("\"{key}\" is declared but not enforced in v1 — the engine records it without acting on it{tail}. Keep it for forward-compatibility or remove it; it changes nothing today."), field, Severity::Warning);
    };
    if let Some(invariants) = spec.invariants.as_ref() {
        if invariants.continuation_required.is_some() {
            v2(issues, "invariants.continuation_required", "workflow.invariants.continuation_required", None);
        }
        if invariants.preserve_lineage.is_some() {
            v2(issues, "invariants.preserve_lineage", "workflow.invariants.preserve_lineage", Some("lineage IS always preserved via chain_of_record; the flag itself gates nothing"));
        }
        if invariants.closure_required.is_some() {
            v2(issues, "invariants.closure_required", "workflow.invariants.closure_required", Some("closure IS always required by the hot-potato contract; the flag itself gates nothing"));
        }
    }
    if spec.closure.is_some() {
        v2(issues, "closure.{success,degraded,failed}", "workflow.closure", Some("display-only messages; no consumer renders them yet"));
    }
    if spec.loop_guards.as_ref().is_some_and(|guards| guards.spawn_budget.is_some()) {
        v2(issues, "loop_guards.spawn_budget", "workflow.loop_guards.spawn_budget", Some("it guards a SPAWN mechanism and no spawn/fan-out seam exists in the single-frontier model; enforcement is a NAMED acceptance item of the WF-2/WF-6 parallel-frontier fan-out work (arch ruling 2026-07-06)"));
    }
    for role in &spec.role_order {
        if spec.roles.get(role).is_some_and(|spec| !spec.skill_refs.is_empty()) {
            v2(issues, "skill_refs", &format!("workflow.roles.{role}.skill_refs"), Some("documentation-only; owner resolution uses preferred_targets"));
            break;
        }
    }
}

fn liveness_issues(
    spec: &WorkflowSpec,
    liveness: Option<&dyn Fn(&str) -> bool>,
    issues: &mut Vec<Issue>,
) {
    let Some(liveness) = liveness else {
        return;
    };
    let names = if spec.role_order.is_empty() {
        spec.roles.keys().cloned().collect()
    } else {
        spec.role_order.clone()
    };
    for role in names {
        let Some(declared) = spec.roles.get(&role) else {
            continue;
        };
        if declared.preferred_targets.is_empty() {
            continue;
        }
        if declared.preferred_targets.iter().any(|target| liveness(target)) {
            continue;
        }
        let rendered = serde_json::to_string(&declared.preferred_targets).unwrap_or_else(|_| "[]".to_owned());
        push(issues, "role_no_live_preferred_target", format!("role \"{role}\" has preferred_targets {rendered} but none are live. The instance may stall at the first step requiring this role; ensure at least one target is up before instantiating, or rely on dynamic resolution at runtime."), format!("workflow.roles.{role}.preferred_targets"), Severity::Warning);
    }
}
