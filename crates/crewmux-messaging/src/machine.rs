// Derived from openrig packages/daemon/src/domain/queue-repository.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Legal state writes. Terminal rows stay put unless `--reopen` names an active state.

use crewmux_types::QItemId;

use crate::closure::{Closure, validate_closure};
use crate::error::Error;
use crate::model::Update;
use crate::state::QueueState;

/// Decide whether `update` may be applied to an item in `from`.
///
/// # Errors
///
/// Returns the TypeScript code and sentence for a reopen, closure, or field mismatch.
pub fn admit(id: &QItemId, from: QueueState, update: &Update) -> Result<Option<Closure>, Error> {
    let Some(to) = update.state else {
        return note_only(update);
    };
    if to == from {
        return note_only(update);
    }
    if from.is_guarded() {
        admit_reopen(id, from, to, update)?;
    }
    admit_fields(to, update)?;
    let closure = validate_closure(
        to.as_str(),
        update.closure_reason.as_deref(),
        target(update),
    )?;
    if to == QueueState::HandedOff {
        require_handoff_close(update)?;
    }
    Ok(closure)
}

fn note_only(update: &Update) -> Result<Option<Closure>, Error> {
    if update.note.trim().is_empty() {
        return Err(Error::Rejected {
            code: "state_or_note_required",
            message: "queue update requires --state or a non-empty --note; nothing was written"
                .to_owned(),
        });
    }
    if update.closure_reason.is_some() || update.closure_target.is_some() || update.reopen {
        return Err(Error::Rejected {
            code: "note_append_fields_not_admitted",
            message: "a state-preserving note append accepts only --note (and an optional same --state); state-write fields were supplied, so nothing was written".to_owned(),
        });
    }
    Ok(None)
}

fn admit_reopen(
    id: &QItemId,
    from: QueueState,
    to: QueueState,
    update: &Update,
) -> Result<(), Error> {
    if !update.reopen {
        return Err(Error::Rejected {
            code: "terminal_reopen_requires_ack",
            message: format!(
                "qitem {id} is currently '{}'; state='{}' would reopen a terminal row. Re-run deliberately with --reopen --note <reason>.",
                from.as_str(),
                to.as_str()
            ),
        });
    }
    if !to.is_active() {
        return Err(Error::Rejected {
            code: "terminal_reopen_target_invalid",
            message: format!(
                "qitem {id} is currently '{}'; --reopen requires an active target state (pending, in-progress, or blocked), not '{}'.",
                from.as_str(),
                to.as_str()
            ),
        });
    }
    if update.note.trim().is_empty() {
        return Err(Error::Rejected {
            code: "reopen_note_required",
            message: format!(
                "qitem {id} is currently '{}'; deliberate reopen requires --note <reason> so the repair is auditable.",
                from.as_str()
            ),
        });
    }
    Ok(())
}

fn admit_fields(to: QueueState, update: &Update) -> Result<(), Error> {
    let admits_closure = to == QueueState::Done
        || (to == QueueState::Blocked && update.closure_reason.as_deref() == Some("blocked_on"))
        || (to == QueueState::HandedOff
            && update.closure_reason.as_deref() == Some("handed_off_to"))
        || (to == QueueState::Canceled && update.closure_reason.as_deref() == Some("superseded"));
    if !admits_closure && (update.closure_reason.is_some() || update.closure_target.is_some()) {
        return Err(Error::Rejected {
            code: "closure_fields_not_admitted",
            message: format!(
                "closure_reason/closure_target persist only on state=done, the blocked park-record form, the handoff close, or the superseded cancel; the '{}' transition cannot store them. Close the item (--state done --closure-reason …) or drop the flags.",
                to.as_str()
            ),
        });
    }
    if update.blocked_on.is_some() && to != QueueState::Blocked {
        return Err(Error::Rejected {
            code: "blocked_on_not_admitted",
            message: format!(
                "blocked_on persists only on state=blocked; the '{}' transition cannot store it. Park the item (rig queue block --on …) or drop --blocked-on.",
                to.as_str()
            ),
        });
    }
    Ok(())
}

fn require_handoff_close(update: &Update) -> Result<(), Error> {
    if update.closure_reason.as_deref() != Some("handed_off_to")
        || target(update).unwrap_or("").is_empty()
    {
        return Err(Error::Rejected {
            code: "missing_closure_target",
            message: "closure_reason=handed_off_to requires closure_target".to_owned(),
        });
    }
    Ok(())
}

fn target(update: &Update) -> Option<&str> {
    update.closure_target.as_deref()
}
