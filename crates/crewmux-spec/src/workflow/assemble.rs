//! Build [`WorkflowSpec`] from a document that has already passed the key checks.

use serde_json::{Map, Value};

use super::error::WorkflowParseError;
use super::model::{
    Acceptance, ClosureText, ExceptionRouting, GateSpec, Invariants, LoopGuards, NextHop, RoleSpec,
    StepSpec, WorkflowSpec,
};
use super::value::{self, object};

pub(crate) fn assemble(
    wf: &Value,
    steps: &[Value],
    mut role_order: Vec<String>,
    source_path: &str,
) -> Result<WorkflowSpec, WorkflowParseError> {
    let id = wf
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let version_value = wf.get("version").unwrap_or(&Value::Null);
    let version_label = value::version_label(version_value);
    let version = u32::try_from(value::integer(version_value).unwrap_or(-1)).unwrap_or(u32::MAX);
    let version = if version_label.bytes().all(|byte| byte.is_ascii_digit()) {
        version_label.parse::<u32>().unwrap_or(version)
    } else if let Some(number) = value::integer(version_value) {
        u32::try_from(number).unwrap_or(u32::MAX)
    } else {
        return Err(WorkflowParseError::new(
            "spec_field_invalid",
            format!(
                "workflow spec at {source_path}: workflow.version must be an integer (got {}).",
                value::js(version_value)
            ),
            Some("workflow.version".to_owned()),
        ));
    };
    let roles_map = wf.get("roles").and_then(object);
    if role_order.is_empty() {
        role_order = roles_map
            .map(|map| map.keys().cloned().collect())
            .unwrap_or_default();
    }
    Ok(WorkflowSpec {
        id,
        version,
        version_label,
        objective: optional_string(wf, "objective"),
        target_rig: wf
            .get("target")
            .and_then(|target| target.get("rig"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        entry_role: wf
            .get("entry")
            .and_then(|entry| entry.get("role"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        coordination_terminal_turn_rule: optional_string(wf, "coordination_terminal_turn_rule"),
        roles: roles_map.map(read_roles).unwrap_or_default(),
        role_order,
        steps: steps.iter().map(read_step).collect(),
        invariants: wf.get("invariants").map(read_invariants),
        closure: wf.get("closure").map(read_closure),
        loop_guards: read_guards(wf.get("loop_guards")),
        exception_routing: wf.get("exception_routing").map(read_routing),
        context_refs: wf.get("context_refs").and_then(value::string_list),
        source_path: source_path.to_owned(),
    })
}

fn optional_string(wf: &Value, key: &str) -> Option<String> {
    wf.get(key).and_then(Value::as_str).map(ToOwned::to_owned)
}

fn read_roles(map: &Map<String, Value>) -> std::collections::BTreeMap<String, RoleSpec> {
    map.iter()
        .map(|(name, role)| (name.clone(), read_role(role)))
        .collect()
}

fn read_role(role: &Value) -> RoleSpec {
    RoleSpec {
        skill_refs: role
            .get("skill_refs")
            .and_then(value::string_list)
            .unwrap_or_default(),
        preferred_targets: role
            .get("preferred_targets")
            .and_then(value::string_list)
            .unwrap_or_default(),
    }
}

fn read_step(step: &Value) -> StepSpec {
    let allowed = step.get("allowed_exits").and_then(value::string_list);
    StepSpec {
        id: step
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        actor_role: step
            .get("actor_role")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        objective: step
            .get("objective")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        allowed_exits: allowed,
        re_present_after_seconds: step
            .get("re_present_after_seconds")
            .and_then(value::integer),
        re_present_max_seconds: step.get("re_present_max_seconds").and_then(value::integer),
        next_hop: step.get("next_hop").map(read_hop),
        harness: step
            .get("harness")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        host: step
            .get("host")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        gate: step.get("gate").and_then(read_gate),
        depends_on: step.get("depends_on").and_then(value::string_list),
        acceptance: step.get("acceptance").and_then(read_acceptance),
    }
}

fn read_hop(hop: &Value) -> NextHop {
    let on = object(hop.get("on").unwrap_or(&Value::Null))
        .map(|map| {
            map.iter()
                .filter_map(|(exit, target)| {
                    target.as_str().map(|id| (exit.clone(), id.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();
    NextHop {
        mode: hop
            .get("mode")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        suggested_roles: hop
            .get("suggested_roles")
            .and_then(value::string_list)
            .unwrap_or_default(),
        on,
    }
}

fn read_gate(gate: &Value) -> Option<GateSpec> {
    Some(GateSpec {
        target: gate.get("target")?.as_str()?.to_owned(),
        summary: gate
            .get("summary")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        evidence_ref: gate
            .get("evidence_ref")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

fn read_acceptance(value: &Value) -> Option<Acceptance> {
    Some(Acceptance {
        candidate: value.get("candidate")?.as_str()?.to_owned(),
        verdicts: value.get("verdicts").and_then(value::string_list)?,
        evidence_ref: value.get("evidence_ref")?.as_str()?.to_owned(),
    })
}

fn read_invariants(value: &Value) -> Invariants {
    Invariants {
        continuation_required: value.get("continuation_required").and_then(Value::as_bool),
        allowed_exits: value.get("allowed_exits").and_then(value::string_list),
        preserve_lineage: value.get("preserve_lineage").and_then(Value::as_bool),
        closure_required: value.get("closure_required").and_then(Value::as_bool),
    }
}

fn read_closure(value: &Value) -> ClosureText {
    ClosureText {
        success: value
            .get("success")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        degraded: value
            .get("degraded")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        failed: value
            .get("failed")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    }
}

fn read_guards(value: Option<&Value>) -> Option<LoopGuards> {
    let map = object(value?)?;
    let max_hops = map
        .get("max_hops")
        .filter(|item| !item.is_null())
        .and_then(value::integer);
    let spawn_budget = map
        .get("spawn_budget")
        .filter(|item| !item.is_null())
        .and_then(value::integer);
    if max_hops.is_none() && spawn_budget.is_none() && map.is_empty() {
        return Some(LoopGuards::default());
    }
    Some(LoopGuards {
        max_hops,
        spawn_budget,
    })
}

fn read_routing(value: &Value) -> ExceptionRouting {
    ExceptionRouting {
        default: value
            .get("default")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        orchestrator_role: value
            .get("orchestrator_role")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        classes: object(value.get("classes").unwrap_or(&Value::Null))
            .map(|map| {
                map.iter()
                    .filter_map(|(key, position)| {
                        position.as_str().map(|text| (key.clone(), text.to_owned()))
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}
