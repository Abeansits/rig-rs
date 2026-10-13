//! Per-step shape checks. `gates` and `mode: prefer` fail before the unknown-key sweep.

use serde_json::Value;

use super::error::WorkflowParseError;
use super::keys::{self, STEP};
use super::model::{AGENT_HARNESSES, EXIT_KINDS};
use super::value::{self, is_mapping, object, reject_unknown};

pub(crate) fn parse_steps<'a>(
    wf: &'a Value,
    source: &str,
) -> Result<&'a Vec<Value>, WorkflowParseError> {
    let steps = wf.get("steps").and_then(Value::as_array).ok_or_else(|| {
        WorkflowParseError::new(
            "spec_field_missing",
            format!("workflow spec at {source} requires at least one step in workflow.steps[]"),
            Some("workflow.steps".to_owned()),
        )
    })?;
    for (index, step) in steps.iter().enumerate() {
        check_step(step, index, source)?;
    }
    Ok(steps)
}

fn check_step(step: &Value, index: usize, source: &str) -> Result<(), WorkflowParseError> {
    if let Some(map) = object(step) {
        check_re_present(map, index, source)?;
        if map.contains_key("gates") {
            return Err(gates_removed(index, source));
        }
        if hop_mode(map) == Some("prefer") {
            return Err(prefer_removed(index, source));
        }
    }
    let path = format!("workflow.steps[{index}]");
    reject_unknown(Some(step), STEP, &path, source)?;
    let Some(map) = object(step) else {
        return Ok(());
    };
    reject_unknown(
        map.get("next_hop"),
        keys::NEXT_HOP,
        &format!("{path}.next_hop"),
        source,
    )?;
    check_branches(map, index, source)?;
    check_harness(map, index, source)?;
    check_host(map, index, source)?;
    check_gate(map, index, source)?;
    check_depends(map, index, source)?;
    check_acceptance(map, index, source)
}

fn hop_mode(step: &serde_json::Map<String, Value>) -> Option<&str> {
    let hop = step.get("next_hop")?;
    if !is_mapping(hop) {
        return None;
    }
    hop.get("mode")?.as_str()
}

fn check_re_present(
    step: &serde_json::Map<String, Value>,
    index: usize,
    source: &str,
) -> Result<(), WorkflowParseError> {
    if let Some(delay) = step.get("re_present_after_seconds") {
        let ok = value::integer(delay).is_some_and(|number| number > 0);
        if !ok {
            return Err(invalid(
                source,
                &format!("workflow.steps[{index}].re_present_after_seconds"),
                &format!(
                    "workflow.steps[{index}].re_present_after_seconds must be a positive integer (got {}). It is the one-shot delay before an intentionally waiting packet is shown to its owner again.",
                    value::js(delay)
                ),
            ));
        }
    }
    if let Some(max) = step.get("re_present_max_seconds") {
        let delay = step.get("re_present_after_seconds").and_then(value::integer);
        let max_n = value::integer(max);
        let ok = max_n.is_some() && delay.is_some_and(|start| max_n.is_some_and(|end| end >= start));
        if !ok {
            return Err(WorkflowParseError::new(
                "spec_field_invalid",
                format!(
                    "workflow spec at {source}: re_present_max_seconds requires re_present_after_seconds and must be an integer at least as large as that initial delay."
                ),
                Some(format!("workflow.steps[{index}].re_present_max_seconds")),
            ));
        }
    }
    Ok(())
}

fn check_branches(
    step: &serde_json::Map<String, Value>,
    index: usize,
    source: &str,
) -> Result<(), WorkflowParseError> {
    let Some(on) = step.get("next_hop").and_then(|hop| hop.get("on")) else {
        return Ok(());
    };
    let Some(map) = object(on) else {
        return Err(invalid(
            source,
            &format!("workflow.steps[{index}].next_hop.on"),
            &format!(
                "workflow.steps[{index}].next_hop.on must be a mapping of recorded exit → step id (got {}).",
                value::js(on)
            ),
        ));
    };
    for (exit_key, target) in map {
        check_branch(exit_key, target, index, source)?;
    }
    Ok(())
}

fn check_branch(
    exit_key: &str,
    target: &Value,
    index: usize,
    source: &str,
) -> Result<(), WorkflowParseError> {
    if !EXIT_KINDS.contains(&exit_key) {
        let allowed = EXIT_KINDS.join(", ");
        return Err(WorkflowParseError::new(
            "spec_branch_key_invalid",
            format!(
                "workflow spec at {source}: workflow.steps[{index}].next_hop.on key \"{exit_key}\" is not a recorded exit. Branch keys are the closed exit enum ONLY: [{allowed}] — branching on anything else (free text, identity, evidence JSON) is deliberately unsupported (branch purity)."
            ),
            Some(format!("workflow.steps[{index}].next_hop.on.{exit_key}")),
        ));
    }
    if target.as_str().is_none_or(str::is_empty) {
        return Err(invalid(
            source,
            &format!("workflow.steps[{index}].next_hop.on.{exit_key}"),
            &format!(
                "workflow.steps[{index}].next_hop.on.{exit_key} must be a step id string (got {}).",
                value::js(target)
            ),
        ));
    }
    Ok(())
}

fn check_harness(
    step: &serde_json::Map<String, Value>,
    index: usize,
    source: &str,
) -> Result<(), WorkflowParseError> {
    let Some(harness) = step.get("harness") else {
        return Ok(());
    };
    if harness.as_str().is_some_and(|name| AGENT_HARNESSES.contains(&name)) {
        return Ok(());
    }
    let allowed = AGENT_HARNESSES.join(", ");
    let terminal = if harness.as_str() == Some("terminal") {
        " A terminal node is not an agent harness — a workflow step cannot be pinned to it."
    } else {
        ""
    };
    Err(WorkflowParseError::new(
        "spec_harness_invalid",
        format!(
            "workflow spec at {source}: workflow.steps[{index}].harness must be one of the AGENT harnesses [{allowed}] (got {}).{terminal} Pi Agent joins the value space in 0.4.7 when its adapter lands.",
            value::js(harness)
        ),
        Some(format!("workflow.steps[{index}].harness")),
    ))
}

fn check_host(
    step: &serde_json::Map<String, Value>,
    index: usize,
    source: &str,
) -> Result<(), WorkflowParseError> {
    let Some(host) = step.get("host") else {
        return Ok(());
    };
    if host.as_str().is_some_and(|text| !text.is_empty()) {
        return Ok(());
    }
    Err(invalid(
        source,
        &format!("workflow.steps[{index}].host"),
        &format!(
            "workflow.steps[{index}].host must be \"local\" or a registered host id string (got {}).",
            value::js(host)
        ),
    ))
}

fn check_gate(
    step: &serde_json::Map<String, Value>,
    index: usize,
    source: &str,
) -> Result<(), WorkflowParseError> {
    let Some(gate) = step.get("gate") else {
        return Ok(());
    };
    if !is_mapping(gate) {
        return Err(invalid(
            source,
            &format!("workflow.steps[{index}].gate"),
            &format!(
                "workflow.steps[{index}].gate must be a mapping with a target (and summary/evidence_ref for human targets). The legacy gates: [...] string list is removed."
            ),
        ));
    }
    reject_unknown(Some(gate), keys::GATE, &format!("workflow.steps[{index}].gate"), source)?;
    let target = gate.get("target").and_then(Value::as_str).unwrap_or("");
    if target.is_empty() {
        return Err(WorkflowParseError::new(
            "spec_field_missing",
            format!(
                "workflow spec at {source}: workflow.steps[{index}].gate.target is required — a human seat session or a declared role name."
            ),
            Some(format!("workflow.steps[{index}].gate.target")),
        ));
    }
    for optional in ["summary", "evidence_ref"] {
        if let Some(value) = gate.get(optional)
            && value.as_str().is_none_or(str::is_empty)
        {
            return Err(invalid(
                source,
                &format!("workflow.steps[{index}].gate.{optional}"),
                &format!(
                    "workflow.steps[{index}].gate.{optional} must be a non-empty string when present (got {}).",
                    value::js(value)
                ),
            ));
        }
    }
    Ok(())
}

fn check_depends(
    step: &serde_json::Map<String, Value>,
    index: usize,
    source: &str,
) -> Result<(), WorkflowParseError> {
    let Some(depends) = step.get("depends_on") else {
        return Ok(());
    };
    let Some(items) = value::string_list(depends) else {
        return Err(invalid(
            source,
            &format!("workflow.steps[{index}].depends_on"),
            &format!(
                "workflow.steps[{index}].depends_on must be a list of non-empty step ids."
            ),
        ));
    };
    if items.iter().any(String::is_empty) {
        return Err(invalid(
            source,
            &format!("workflow.steps[{index}].depends_on"),
            &format!(
                "workflow.steps[{index}].depends_on must be a list of non-empty step ids."
            ),
        ));
    }
    let mut seen = Vec::new();
    for item in &items {
        if seen.contains(&item.as_str()) {
            return Err(invalid(
                source,
                &format!("workflow.steps[{index}].depends_on"),
                &format!(
                    "workflow.steps[{index}].depends_on contains a duplicate prerequisite."
                ),
            ));
        }
        seen.push(item.as_str());
    }
    Ok(())
}

fn check_acceptance(
    step: &serde_json::Map<String, Value>,
    index: usize,
    source: &str,
) -> Result<(), WorkflowParseError> {
    let Some(acceptance) = step.get("acceptance") else {
        return Ok(());
    };
    if !is_mapping(acceptance) {
        return Err(invalid(
            source,
            &format!("workflow.steps[{index}].acceptance"),
            &format!(
                "workflow.steps[{index}].acceptance must be a mapping with candidate, verdicts, and evidence_ref."
            ),
        ));
    }
    reject_unknown(
        Some(acceptance),
        keys::ACCEPTANCE,
        &format!("workflow.steps[{index}].acceptance"),
        source,
    )?;
    if acceptance
        .get("candidate")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        return Err(missing_field(
            source,
            &format!("workflow.steps[{index}].acceptance.candidate"),
            &format!("workflow.steps[{index}].acceptance.candidate is required."),
        ));
    }
    let verdicts = acceptance.get("verdicts").and_then(value::string_list);
    let verdicts_ok = verdicts.is_some_and(|items| !items.is_empty() && items.iter().all(|item| !item.is_empty()));
    if !verdicts_ok {
        return Err(invalid(
            source,
            &format!("workflow.steps[{index}].acceptance.verdicts"),
            &format!(
                "workflow.steps[{index}].acceptance.verdicts must be a non-empty list of verdict strings."
            ),
        ));
    }
    if acceptance
        .get("evidence_ref")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        return Err(missing_field(
            source,
            &format!("workflow.steps[{index}].acceptance.evidence_ref"),
            &format!("workflow.steps[{index}].acceptance.evidence_ref is required."),
        ));
    }
    Ok(())
}

fn gates_removed(index: usize, source: &str) -> WorkflowParseError {
    WorkflowParseError::new(
        "spec_gates_removed",
        format!(
            "workflow spec at {source}: workflow.steps[{index}].gates is removed in 0.4.6. The string list could not carry a target/summary/evidence without becoming a magic-string mini-grammar, and it was never enforced. Declare the structured step-level gate instead:\n  gate:\n    target: <human seat session or declared role name>\n    summary: <plain-language ask>          # required for a human target\n    evidence_ref: <durable artifact path>  # required for a human target"
        ),
        Some(format!("workflow.steps[{index}].gates")),
    )
}

fn prefer_removed(index: usize, source: &str) -> WorkflowParseError {
    WorkflowParseError::new(
        "spec_prefer_mode_removed",
        format!(
            "workflow spec at {source}: workflow.steps[{index}].next_hop.mode \"prefer\" is removed in 0.4.6. It never had distinct behavior — routing treated it identically to omitting mode. Delete the mode line (same routing), or use \"require\" (route ONLY via suggested_roles; no declaration-order fallback) / \"forbid\" (terminal step)."
        ),
        Some(format!("workflow.steps[{index}].next_hop.mode")),
    )
}

fn invalid(source: &str, field: &str, sentence: &str) -> WorkflowParseError {
    WorkflowParseError::new(
        "spec_field_invalid",
        format!("workflow spec at {source}: {sentence}"),
        Some(field.to_owned()),
    )
}

fn missing_field(source: &str, field: &str, sentence: &str) -> WorkflowParseError {
    WorkflowParseError::new(
        "spec_field_missing",
        format!("workflow spec at {source}: {sentence}"),
        Some(field.to_owned()),
    )
}
