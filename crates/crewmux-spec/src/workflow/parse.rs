//! `parseWorkflowSpec`. Messages and codes match the TypeScript cache.

use serde_json::Value;

use crate::yaml::parse_yaml;

use super::error::WorkflowParseError;
use super::keys::{self, TOP};
use super::model::WorkflowSpec;
use super::order::role_order;
use super::steps::parse_steps;
use super::value::{self, is_mapping, object, reject_unknown};

/// Parse `text` as a workflow spec wrapped in a top-level `workflow:` key.
///
/// # Errors
///
/// Returns [`WorkflowParseError`] with the TypeScript code and message.
pub fn parse_workflow(text: &str, source_path: &str) -> Result<WorkflowSpec, WorkflowParseError> {
    let parsed: Value = parse_yaml(text).map_err(|err| {
        WorkflowParseError::new(
            "spec_yaml_invalid",
            format!(
                "workflow spec at {source_path} could not be parsed as YAML: {}",
                err.message
            ),
            None,
        )
    })?;
    let root = mapping_root(&parsed, source_path)?;
    let wf = workflow_node(root, source_path)?;
    require_identity(wf, source_path)?;
    reject_unknown(Some(&parsed), &["workflow"], "(document root)", source_path)?;
    reject_unknown(Some(wf), TOP, "workflow", source_path)?;
    check_loop_guards(wf, source_path)?;
    check_routing(wf, source_path)?;
    reject_unknown(wf.get("target"), keys::TARGET, "workflow.target", source_path)?;
    reject_unknown(wf.get("entry"), keys::ENTRY, "workflow.entry", source_path)?;
    reject_unknown(
        wf.get("invariants"),
        keys::INVARIANTS,
        "workflow.invariants",
        source_path,
    )?;
    reject_unknown(
        wf.get("closure"),
        keys::CLOSURE,
        "workflow.closure",
        source_path,
    )?;
    reject_unknown(
        wf.get("loop_guards"),
        keys::LOOP,
        "workflow.loop_guards",
        source_path,
    )?;
    check_roles(wf, source_path)?;
    check_context_refs(wf, source_path)?;
    let steps = parse_steps(wf, source_path)?;
    super::assemble::assemble(wf, steps, role_order(text), source_path)
}

fn mapping_root<'a>(
    parsed: &'a Value,
    source_path: &str,
) -> Result<&'a serde_json::Map<String, Value>, WorkflowParseError> {
    object(parsed).ok_or_else(|| {
        WorkflowParseError::new(
            "spec_shape_invalid",
            format!(
                "workflow spec at {source_path} must be a YAML mapping with a top-level 'workflow:' key"
            ),
            None,
        )
    })
}

fn workflow_node<'a>(
    root: &'a serde_json::Map<String, Value>,
    source_path: &str,
) -> Result<&'a Value, WorkflowParseError> {
    match root.get("workflow") {
        Some(wf) if is_mapping(wf) => Ok(wf),
        _ => Err(WorkflowParseError::new(
            "spec_shape_invalid",
            format!("workflow spec at {source_path} is missing the top-level 'workflow:' key"),
            None,
        )),
    }
}

fn require_identity(
    wf: &Value,
    source_path: &str,
) -> Result<(), WorkflowParseError> {
    let id = wf.get("id").and_then(Value::as_str).unwrap_or("");
    if id.is_empty() {
        return Err(missing(source_path, "workflow.id", "is missing required field workflow.id"));
    }
    if wf.get("version").is_none_or(Value::is_null) {
        return Err(missing(
            source_path,
            "workflow.version",
            "is missing required field workflow.version",
        ));
    }
    let steps = wf.get("steps").and_then(Value::as_array);
    if steps.is_none_or(Vec::is_empty) {
        return Err(missing(
            source_path,
            "workflow.steps",
            "requires at least one step in workflow.steps[]",
        ));
    }
    if !wf.get("roles").is_some_and(is_mapping) {
        return Err(missing(
            source_path,
            "workflow.roles",
            "requires a workflow.roles mapping",
        ));
    }
    Ok(())
}

fn missing(source: &str, field: &str, tail: &str) -> WorkflowParseError {
    WorkflowParseError::new(
        "spec_field_missing",
        format!("workflow spec at {source} {tail}"),
        Some(field.to_owned()),
    )
}

fn check_roles(wf: &Value, source: &str) -> Result<(), WorkflowParseError> {
    let Some(roles) = wf.get("roles").and_then(object) else {
        return Ok(());
    };
    for (name, role) in roles {
        reject_unknown(
            Some(role),
            keys::ROLE,
            &format!("workflow.roles.{name}"),
            source,
        )?;
    }
    Ok(())
}

fn check_context_refs(wf: &Value, source: &str) -> Result<(), WorkflowParseError> {
    let Some(refs) = wf.get("context_refs") else {
        return Ok(());
    };
    let bad = !refs
        .as_array()
        .is_some_and(|items| items.iter().all(|item| item.as_str().is_some_and(|text| !text.trim().is_empty())));
    if bad {
        return Err(WorkflowParseError::new(
            "spec_field_invalid",
            format!("workflow spec at {source}: context_refs must be non-empty string addresses."),
            Some("workflow.context_refs".to_owned()),
        ));
    }
    Ok(())
}

fn check_loop_guards(wf: &Value, source: &str) -> Result<(), WorkflowParseError> {
    let Some(guards) = wf.get("loop_guards").and_then(object) else {
        return Ok(());
    };
    if let Some(max_hops) = guards.get("max_hops").filter(|value| !value.is_null()) {
        let ok = value::integer(max_hops).is_some_and(|number| number >= 1);
        if !ok {
            return Err(invalid(
                source,
                "workflow.loop_guards.max_hops",
                &format!(
                    "workflow.loop_guards.max_hops must be an integer >= 1 (got {}). A non-numeric or non-positive guard can never trip at projection, so it cannot sanction a cycle — fix the value or remove the key.",
                    value::js(max_hops)
                ),
            ));
        }
    }
    if let Some(budget) = guards.get("spawn_budget").filter(|value| !value.is_null()) {
        let ok = value::integer(budget).is_some_and(|number| number >= 0);
        if !ok {
            return Err(invalid(
                source,
                "workflow.loop_guards.spawn_budget",
                &format!(
                    "workflow.loop_guards.spawn_budget must be an integer >= 0 (got {}).",
                    value::js(budget)
                ),
            ));
        }
    }
    Ok(())
}

fn check_routing(wf: &Value, source: &str) -> Result<(), WorkflowParseError> {
    let Some(routing) = wf.get("exception_routing") else {
        return Ok(());
    };
    if !is_mapping(routing) {
        return Err(invalid(
            source,
            "workflow.exception_routing",
            &format!(
                "workflow.exception_routing must be a mapping (got {}). Declare default / orchestrator_role / classes, or remove the key for the host-default → orchestrator-first chain.",
                value::js(routing)
            ),
        ));
    }
    reject_unknown(
        Some(routing),
        keys::ROUTING,
        "workflow.exception_routing",
        source,
    )?;
    check_routing_fields(routing, source)
}

fn check_routing_fields(routing: &Value, source: &str) -> Result<(), WorkflowParseError> {
    if let Some(default) = routing.get("default")
        && !is_position(default)
    {
        return Err(invalid(
            source,
            "workflow.exception_routing.default",
            &format!(
                "workflow.exception_routing.default must be \"orchestrator\" or \"human_only\" (got {}).",
                value::js(default)
            ),
        ));
    }
    if let Some(role) = routing.get("orchestrator_role")
        && role.as_str().is_none_or(str::is_empty)
    {
        return Err(invalid(
            source,
            "workflow.exception_routing.orchestrator_role",
            &format!(
                "workflow.exception_routing.orchestrator_role must be a non-empty declared role name (got {}). Role EXISTENCE is the validator's graph check.",
                value::js(role)
            ),
        ));
    }
    check_classes(routing.get("classes"), source)
}

fn check_classes(classes: Option<&Value>, source: &str) -> Result<(), WorkflowParseError> {
    let Some(classes) = classes else {
        return Ok(());
    };
    let Some(map) = object(classes) else {
        return Err(invalid(
            source,
            "workflow.exception_routing.classes",
            &format!(
                "workflow.exception_routing.classes must be a mapping of exception class → position (got {}).",
                value::js(classes)
            ),
        ));
    };
    for (key, value) in map {
        check_class(key, value, source)?;
    }
    Ok(())
}

fn check_class(key: &str, value: &Value, source: &str) -> Result<(), WorkflowParseError> {
    let field = format!("workflow.exception_routing.classes.{key}");
    if key == "human_gate_trip" {
        return Err(invalid(
            source,
            &field,
            "workflow.exception_routing.classes.human_gate_trip is not configurable — a human gate is intrinsically human-only (the human decision IS the exception); the dial cannot re-point it. Remove the line.",
        ));
    }
    if key != "unmapped_failed" && key != "stuck_overdue" {
        return Err(WorkflowParseError::new(
            "spec_unknown_key",
            format!(
                "workflow spec at {source}: workflow.exception_routing.classes.{key} is not a known exception class. Allowed: unmapped_failed, stuck_overdue."
            ),
            Some(field),
        ));
    }
    if !is_position(value) {
        return Err(invalid(
            source,
            &field,
            &format!(
                "workflow.exception_routing.classes.{key} must be \"orchestrator\" or \"human_only\" (got {}).",
                value::js(value)
            ),
        ));
    }
    Ok(())
}

fn is_position(value: &Value) -> bool {
    matches!(value.as_str(), Some("orchestrator" | "human_only"))
}

fn invalid(source: &str, field: &str, sentence: &str) -> WorkflowParseError {
    WorkflowParseError::new(
        "spec_field_invalid",
        format!("workflow spec at {source}: {sentence}"),
        Some(field.to_owned()),
    )
}
