// Derived from openrig packages/daemon/src/domain/workflow-projector.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Packet bodies. The continuation line is the project command, not a paste.

use crewmux_spec::workflow::{StepSpec, WorkflowSpec};

const SHORTCUT: &str = "Context: this workflow packet is a shortcut, not the whole story; inspect additional project, mission, and receipt evidence before choosing an authored exit.";

/// `rig workflow project` line embedded in a packet.
#[must_use]
pub(crate) fn project_command(instance: &str, packet: &str, owner: &str) -> String {
    format!(
        "rig workflow project --instance {instance} --current-packet {packet} --exit <handoff|waiting|done|failed> --actor-session {owner}"
    )
}

/// Entry packet body.
#[must_use]
pub(crate) fn entry_body(
    spec: &WorkflowSpec,
    instance: &str,
    step: &StepSpec,
    objective: &str,
    packet: &str,
    owner: &str,
) -> String {
    let mut lines = vec![
        format!(
            "### Workflow entry: {}@{} step {}",
            spec.id, spec.version_label, step.id
        ),
        String::new(),
        format!("Workflow instance: {instance}"),
        format!("Entry step: {} ({})", step.id, step.actor_role),
        String::new(),
        format!("Root objective: {objective}"),
    ];
    if let Some(text) = &step.objective {
        lines.push(String::new());
        lines.push(format!("Step objective: {text}"));
    }
    finish(lines, instance, packet, owner)
}

/// Handoff packet body.
#[must_use]
#[expect(
    clippy::too_many_arguments,
    reason = "the handoff body quotes the prior step, the next step, and the continuation"
)]
pub(crate) fn handoff_body(
    spec: &WorkflowSpec,
    instance: &str,
    current: &StepSpec,
    next: &StepSpec,
    actor: &str,
    note: Option<&str>,
    packet: &str,
    owner: &str,
) -> String {
    let mut lines = vec![
        format!(
            "### Workflow handoff: {}@{} step {}",
            spec.id, spec.version_label, next.id
        ),
        String::new(),
        format!("Workflow instance: {instance}"),
        format!(
            "Prior step: {} ({}) closed by {actor}",
            current.id, current.actor_role
        ),
        format!("This step: {} ({})", next.id, next.actor_role),
    ];
    if let Some(text) = &next.objective {
        lines.push(format!("Objective: {text}"));
    }
    if let Some(note) = note {
        lines.push(String::new());
        lines.push(format!("Prior step note: {note}"));
    }
    finish(lines, instance, packet, owner)
}

fn finish(mut lines: Vec<String>, instance: &str, packet: &str, owner: &str) -> String {
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.push(String::new());
    lines.push(format!(
        "Continuation: {}",
        project_command(instance, packet, owner)
    ));
    lines.push(SHORTCUT.to_owned());
    lines.join("\n")
}
