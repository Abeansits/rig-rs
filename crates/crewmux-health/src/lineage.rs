// Derived from openrig packages/daemon/src/domain/health-detectors.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Wake-lineage replay over queue transition notes.

use std::collections::BTreeMap;

use crewmux_core::Timestamp;
use crewmux_types::QItemId;

use crate::coverage::Coverage;
use crate::observation::HealthObservation;
use messaging_notes::NoteGroup;

/// One queue transition note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransitionNote {
    /// Queue item id.
    pub lineage_id: QItemId,
    /// Transition note. Ladder markers use the TypeScript prefixes.
    pub note: String,
    /// The queue row already has an obligation body. Wakes then duplicate it.
    pub next_action: bool,
}

/// Observations and coverage for the wake-lineage replay.
///
/// A lineage past `limit` is omitted and is not reported healthy.
#[must_use]
pub fn wake_from_notes(
    notes: &[TransitionNote],
    limit: u32,
    at: Timestamp,
) -> (Vec<HealthObservation>, Coverage) {
    let mut groups: BTreeMap<QItemId, NoteGroup> = BTreeMap::new();
    for note in notes {
        if !is_wake_note(&note.note) {
            continue;
        }
        let group = groups.entry(note.lineage_id.clone()).or_default();
        if note.next_action {
            group.next_action = true;
        }
        group.observe(&note.note);
    }
    let total = u32::try_from(groups.len()).unwrap_or(u32::MAX);
    let keep = usize::try_from(limit).unwrap_or(usize::MAX);
    let mut observations = Vec::new();
    for (index, (lineage_id, group)) in groups.into_iter().enumerate() {
        if index >= keep {
            break;
        }
        observations.push(HealthObservation::WakeLineage {
            lineage_id: lineage_id.as_str().to_owned(),
            failed: group.failed,
            escalated: group.escalated,
            wake_count: group.wake_count,
            rescue_wake_count: group.rescue_wake_count,
            existing_next_action: group.next_action,
            at,
        });
    }
    let evaluated = u32::try_from(observations.len()).unwrap_or(u32::MAX);
    let omitted = total.saturating_sub(evaluated);
    (
        observations,
        Coverage {
            evaluated,
            omitted,
            limit_hit: omitted > 0,
        },
    )
}

fn is_wake_note(note: &str) -> bool {
    let text = outside_quotes(note);
    text.contains("wake-attempt:") || text.contains("escalation-rung:")
}

/// Drop single- and double-quoted spans so a cited phrase is not a marker.
fn outside_quotes(note: &str) -> String {
    let mut out = String::new();
    let mut quote = None;
    for ch in note.chars() {
        if let Some(mark) = quote {
            if ch == mark {
                quote = None;
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            continue;
        }
        out.push(ch);
    }
    out
}

mod messaging_notes {
    #[derive(Default)]
    pub(super) struct NoteGroup {
        pub(super) failed: bool,
        pub(super) escalated: bool,
        pub(super) wake_count: u32,
        pub(super) rescue_wake_count: u32,
        pub(super) next_action: bool,
    }

    impl NoteGroup {
        pub(super) fn observe(&mut self, note: &str) {
            let text = super::outside_quotes(note);
            if text.contains("escalation-rung:") {
                self.escalated = true;
            }
            if text.contains("wake-attempt:") {
                self.wake_count = self.wake_count.saturating_add(1);
                if text.contains("outcome=failed") {
                    self.failed = true;
                }
                if text.contains("outcome=rescue") {
                    self.rescue_wake_count = self.rescue_wake_count.saturating_add(1);
                }
            }
        }
    }
}
