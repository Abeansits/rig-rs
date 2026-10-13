// Derived from openrig packages/daemon/src/domain/workflow-projector.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Next-step and owner resolution. The validator walks this function.

use crewmux_spec::workflow::{StepSpec, WorkflowSpec};

use crate::error::Error;
use crate::role::{SeatCandidate, select_role_seat};
use crate::world::World;

/// Structural or branch successor.
///
/// A mapped `exit` wins and does not fall through, even when the target
/// step is missing. `exit` of `None` is the structural walk.
#[must_use]
pub fn resolve_next_step<'a>(
    spec: &'a WorkflowSpec,
    step: &StepSpec,
    exit: Option<&str>,
) -> Option<&'a StepSpec> {
    if let Some(exit) = exit
        && let Some(hop) = step.next_hop.as_ref()
        && let Some(target) = hop.mapped(exit)
    {
        return spec.steps.iter().find(|candidate| candidate.id == target);
    }
    if step.next_hop.as_ref().is_some_and(|hop| hop.mode.as_deref() == Some("forbid")) {
        return None;
    }
    if let Some(hop) = step.next_hop.as_ref() {
        for role in &hop.suggested_roles {
            if let Some(found) = spec.steps.iter().find(|candidate| candidate.actor_role == *role) {
                return Some(found);
            }
        }
        if hop.mode.as_deref() == Some("require") {
            return None;
        }
    }
    let index = spec.steps.iter().position(|candidate| candidate.id == step.id)?;
    spec.steps.get(index.saturating_add(1))
}

/// Owner for `step`. Preferred targets win in order and are not liveness-filtered.
///
/// # Errors
///
/// Returns a harness or bound-rig rejection. `Ok(None)` is the unbound role
/// with no preferred targets; the caller names `next_owner_unresolved`.
pub(crate) fn resolve_owner(
    spec: &WorkflowSpec,
    step: &StepSpec,
    world: &World,
    bound: bool,
) -> Result<Option<String>, Error> {
    let targets = spec
        .roles
        .get(&step.actor_role)
        .map(|role| role.preferred_targets.clone())
        .unwrap_or_default();
    if targets.is_empty() && bound {
        return Ok(Some(resolve_on_bound_rig(step, world)?));
    }
    if step.harness.is_none() {
        return Ok(targets.first().cloned());
    }
    let harness = step.harness.as_deref().unwrap_or("");
    let candidates: Vec<(String, Option<String>)> = targets
        .iter()
        .map(|target| (target.clone(), world.runtime_of(target)))
        .collect();
    if let Some((session, _)) = candidates
        .iter()
        .find(|(_, runtime)| runtime.as_deref() == Some(harness))
    {
        return Ok(Some(session.clone()));
    }
    let listed = if candidates.is_empty() {
        "(none declared)".to_owned()
    } else {
        candidates
            .iter()
            .map(|(session, runtime)| {
                format!("{} → {}", session, runtime.as_deref().unwrap_or("unknown"))
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    Err(Error::reject(
        "harness_pin_unsatisfied",
        format!(
            "step \"{}\" pins harness \"{harness}\" but no preferred_target of role \"{}\" runs it. Candidates: {listed}. Add a {harness} seat to the role's preferred_targets, or change/remove the pin.",
            step.id, step.actor_role
        ),
        409,
    ))
}

/// An explicit owner must match a harness pin.
///
/// # Errors
///
/// Returns [`Error`] `harness_pin_unsatisfied` when the runtime differs.
pub(crate) fn reconcile_explicit(step: &StepSpec, owner: &str, world: &World) -> Result<(), Error> {
    let Some(harness) = step.harness.as_deref() else {
        return Ok(());
    };
    let actual = world.runtime_of(owner);
    if actual.as_deref() == Some(harness) {
        return Ok(());
    }
    Err(Error::reject(
        "harness_pin_unsatisfied",
        format!(
            "step \"{}\" pins harness \"{harness}\" but the explicitly supplied owner {owner} runs {}. Supply a {harness} seat or change/remove the pin.",
            step.id,
            actual.as_deref().unwrap_or("unknown (not a managed node)")
        ),
        409,
    ))
}

fn resolve_on_bound_rig(step: &StepSpec, world: &World) -> Result<String, Error> {
    let Some(rig) = world.bound_seats.as_ref() else {
        let rig_name = world.bound_rig.as_deref().unwrap_or("");
        return Err(Error::reject(
            "bound_rig_not_found",
            format!(
                "cannot resolve role \"{}\" for step \"{}\": the instance's bound rig \"{rig_name}\" no longer resolves to a registered rig (torn down mid-run?). Recreate/import the rig under the same name, or instantiate a fresh run bound elsewhere.",
                step.actor_role, step.id
            ),
            409,
        ));
    };
    let pick = select_role_seat(
        &step.actor_role,
        step.harness.as_deref(),
        rig,
    );
    if let Some(seat) = pick.seat {
        return Ok(seat);
    }
    let declaring: Vec<&SeatCandidate> = rig
        .iter()
        .filter(|seat| seat.role.as_deref() == Some(step.actor_role.as_str()))
        .collect();
    if declaring.is_empty() {
        return Err(Error::reject(
            "next_owner_unresolved",
            format!(
                "cannot resolve role \"{}\" for step \"{}\" (no seats on the bound rig declare role \"{}\"). Add a member with role {} to the rig via rig add, declare role on an existing member, or add preferred_targets to the role in the spec.",
                step.actor_role, step.id, step.actor_role, step.actor_role
            ),
            400,
        ));
    }
    let listed = pick
        .disqualified
        .iter()
        .filter(|row| {
            rig.iter().any(|seat| {
                seat.logical_id == row.logical_id && seat.role.as_deref() == Some(step.actor_role.as_str())
            })
        })
        .map(|row| {
            format!(
                "{} → {}",
                row.coordinate.as_deref().unwrap_or(row.logical_id.as_str()),
                row.disqualifier
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    Err(Error::reject(
        "next_owner_unresolved",
        format!(
            "cannot resolve a live capable seat for role \"{}\" for step \"{}\" on the bound rig. Candidates: {listed}. Start/repair a role seat (see rig ps), add a member with role {} via rig add, or add preferred_targets to the role.",
            step.actor_role, step.id, step.actor_role
        ),
        400,
    ))
}
