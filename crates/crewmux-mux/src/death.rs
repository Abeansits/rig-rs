//! Pane-death dedupe. One `PaneDied` per `(pane id, spawn generation)`.

use std::collections::{HashMap, HashSet};

/// Tracks which pane deaths have already been emitted.
#[derive(Debug, Default)]
pub(crate) struct DeathDedupe {
    generation: HashMap<String, u64>,
    emitted: HashSet<(String, u64)>,
}

impl DeathDedupe {
    /// Generation for a pane, assigning 1 the first time it is seen.
    pub(crate) fn generation(&mut self, pane_id: &str) -> u64 {
        if let Some(generation) = self.generation.get(pane_id) {
            return *generation;
        }
        self.generation.insert(pane_id.to_owned(), 1);
        1
    }

    /// Bump the generation after the pane is alive again (a new spawn).
    pub(crate) fn note_alive(&mut self, pane_id: &str) {
        let next = self
            .generation
            .get(pane_id)
            .copied()
            .unwrap_or(0)
            .saturating_add(1);
        // The first observation is generation 1 via `generation`. A later alive
        // note after a death advances so the next death can emit.
        if self.emitted.iter().any(|(id, _)| id == pane_id) {
            self.generation.insert(pane_id.to_owned(), next);
            self.emitted.retain(|(id, _)| id != pane_id);
        } else {
            self.generation.entry(pane_id.to_owned()).or_insert(1);
        }
    }

    /// `true` when this death should be emitted.
    pub(crate) fn emit(&mut self, pane_id: &str) -> bool {
        let generation = self.generation(pane_id);
        self.emitted.insert((pane_id.to_owned(), generation))
    }
}

#[cfg(test)]
mod tests {
    use super::DeathDedupe;

    #[test]
    fn pane_died_is_deduped_until_the_next_spawn() {
        let mut dedupe = DeathDedupe::default();
        assert!(dedupe.emit("%1"));
        assert!(
            !dedupe.emit("%1"),
            "doorbell and backstop share one emission"
        );
        dedupe.note_alive("%1");
        assert!(dedupe.emit("%1"), "a new spawn generation emits again");
        assert!(dedupe.emit("%2"));
    }
}
