// Derived from openrig packages/daemon/src/domain/restore-orchestrator.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Launch waves. Only `delegates_to` and `spawned_by` constrain order.

use std::collections::HashMap;

use crewmux_types::SeatId;

use crate::error::Error;

/// One recorded edge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchEdge {
    /// Source seat.
    pub source: SeatId,
    /// Target seat.
    pub target: SeatId,
    /// Kind token. Other kinds are ignored.
    pub kind: String,
}

/// Seats grouped into waves. Within a wave, names sort ascending.
///
/// `delegates_to`: source launches before target.
/// `spawned_by`: target (parent) launches before source (child).
///
/// # Errors
///
/// Returns [`Error::Cycle`] when the constraining edges loop.
pub fn launch_waves(
    seats: &[(SeatId, &str)],
    edges: &[LaunchEdge],
) -> Result<Vec<Vec<SeatId>>, Error> {
    let mut names: HashMap<SeatId, &str> = HashMap::new();
    for (id, name) in seats {
        names.insert(*id, *name);
    }
    let mut indegree: HashMap<SeatId, usize> = names.keys().map(|id| (*id, 0)).collect();
    let mut next: HashMap<SeatId, Vec<SeatId>> = names.keys().map(|id| (*id, Vec::new())).collect();
    for edge in edges {
        let (from, to) = match edge.kind.as_str() {
            "delegates_to" => (edge.source, edge.target),
            "spawned_by" => (edge.target, edge.source),
            _ => continue,
        };
        if !names.contains_key(&from) || !names.contains_key(&to) {
            continue;
        }
        if let Some(list) = next.get_mut(&from) {
            list.push(to);
        }
        if let Some(count) = indegree.get_mut(&to) {
            *count = count.saturating_add(1);
        }
    }
    let mut remaining = names.len();
    let mut waves = Vec::new();
    while remaining > 0 {
        let mut wave: Vec<SeatId> = indegree
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(id, _)| *id)
            .collect();
        if wave.is_empty() {
            return Err(Error::Cycle);
        }
        wave.sort_by(|left, right| name_of(&names, left).cmp(name_of(&names, right)));
        for id in &wave {
            indegree.remove(id);
            let children = next.remove(id).unwrap_or_default();
            for child in children {
                if let Some(count) = indegree.get_mut(&child) {
                    *count = count.saturating_sub(1);
                }
            }
        }
        remaining = remaining.saturating_sub(wave.len());
        waves.push(wave);
    }
    Ok(waves)
}

fn name_of<'a>(names: &HashMap<SeatId, &'a str>, id: &SeatId) -> &'a str {
    names.get(id).copied().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::{LaunchEdge, launch_waves};
    use crewmux_types::SeatId;

    fn ids() -> (SeatId, SeatId, SeatId) {
        (SeatId::new(), SeatId::new(), SeatId::new())
    }

    #[test]
    fn delegates_to_orders_the_source_first() {
        let (alpha, beta, gamma) = ids();
        let seats = [(gamma, "c"), (alpha, "a"), (beta, "b")];
        let edges = [LaunchEdge {
            source: alpha,
            target: beta,
            kind: "delegates_to".to_owned(),
        }];
        let waves = launch_waves(&seats, &edges).expect("waves");
        assert_eq!(waves.len(), 2);
        assert_eq!(
            waves.first().map(Vec::as_slice),
            Some([alpha, gamma].as_slice())
        );
        assert_eq!(waves.get(1).map(Vec::as_slice), Some([beta].as_slice()));
    }

    #[test]
    fn spawned_by_orders_the_parent_first() {
        let (child, parent, _) = ids();
        let seats = [(child, "child"), (parent, "parent")];
        let edges = [LaunchEdge {
            source: child,
            target: parent,
            kind: "spawned_by".to_owned(),
        }];
        let waves = launch_waves(&seats, &edges).expect("waves");
        assert_eq!(waves.first().and_then(|wave| wave.first()), Some(&parent));
        assert_eq!(waves.get(1).and_then(|wave| wave.first()), Some(&child));
    }

    #[test]
    fn can_observe_does_not_constrain_order() {
        let (left, right, _) = ids();
        let seats = [(right, "b"), (left, "a")];
        let edges = [LaunchEdge {
            source: right,
            target: left,
            kind: "can_observe".to_owned(),
        }];
        let waves = launch_waves(&seats, &edges).expect("waves");
        assert_eq!(waves.len(), 1);
        assert_eq!(
            waves.first().map(Vec::as_slice),
            Some([left, right].as_slice())
        );
    }
}
