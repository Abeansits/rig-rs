// Derived from openrig packages/daemon/src/domain/active-occupant.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Four-way active-occupant ladder.
//!
//! A missing map is the only case where legacy inference may run. A present
//! map is the authority: a null, a missing key, or a dangling id is ambiguous.

use serde::{Deserialize, Serialize};

/// One session row the ladder can see.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OccupantRow {
    /// Session id.
    pub id: String,
    /// Seat or node id.
    pub node_id: String,
    /// `running` is the live status. Anything else is historical.
    pub status: String,
}

/// Explicit capture of who occupied a seat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OccupantState {
    /// No live row.
    Absent,
    /// Exactly one live session.
    Resolved {
        /// Session id.
        session_id: String,
    },
    /// More than one live candidate.
    Ambiguous {
        /// Candidate session ids.
        candidate_ids: Vec<String>,
    },
}

/// What the ladder decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OccupantResolution {
    /// One session.
    Resolved(OccupantRow),
    /// The seat never ran (legacy map only).
    None,
    /// Fail closed. Do not pick the newest row.
    Ambiguous {
        /// Candidate ids.
        candidate_ids: Vec<String>,
        /// Why.
        detail: String,
    },
}

/// Resolve the occupant for `node_id`.
///
/// `relation` `None` is a pre-convention snapshot. `Some` is authoritative
/// even when the value for this node is missing.
#[must_use]
pub fn resolve_active_occupant(
    sessions: &[OccupantRow],
    relation: Option<&std::collections::BTreeMap<String, Option<String>>>,
    node_id: &str,
) -> OccupantResolution {
    let rows: Vec<&OccupantRow> = sessions
        .iter()
        .filter(|row| row.node_id == node_id)
        .collect();
    let Some(map) = relation else {
        return legacy(&rows);
    };
    let Some(rel) = map.get(node_id) else {
        return ambiguous(
            &rows,
            "the snapshot's activeSessionIdByNode map carries no entry for this node",
        );
    };
    let Some(session_id) = rel else {
        return ambiguous(
            &rows,
            "the snapshot recorded no single live occupant at capture (explicit null)",
        );
    };
    match rows.iter().find(|row| row.id == *session_id) {
        Some(row) => OccupantResolution::Resolved((*row).clone()),
        None => ambiguous(
            &rows,
            &format!(
                "the recorded active relation {session_id} names no session row in this snapshot (dangling)"
            ),
        ),
    }
}

fn legacy(rows: &[&OccupantRow]) -> OccupantResolution {
    match rows {
        [] => OccupantResolution::None,
        [only] => OccupantResolution::Resolved((*only).clone()),
        many => {
            let running: Vec<&&OccupantRow> =
                many.iter().filter(|row| row.status == "running").collect();
            if let [only] = running.as_slice() {
                OccupantResolution::Resolved((**only).clone())
            } else {
                ambiguous(
                    many,
                    "no explicit active relation (pre-convention snapshot) and no uniquely-running row",
                )
            }
        }
    }
}

fn ambiguous(rows: &[&OccupantRow], detail: &str) -> OccupantResolution {
    OccupantResolution::Ambiguous {
        candidate_ids: rows.iter().map(|row| row.id.clone()).collect(),
        detail: detail.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{OccupantResolution, OccupantRow, resolve_active_occupant};
    use std::collections::BTreeMap;

    fn row(id: &str, status: &str) -> OccupantRow {
        OccupantRow {
            id: id.to_owned(),
            node_id: "seat".to_owned(),
            status: status.to_owned(),
        }
    }

    #[test]
    fn four_way_ladder_fails_closed_on_a_present_map() {
        let sessions = [
            row("older-running", "running"),
            row("newer-exited", "exited"),
        ];
        match resolve_active_occupant(&sessions, None, "seat") {
            OccupantResolution::Resolved(found) => assert_eq!(found.id, "older-running"),
            OccupantResolution::None | OccupantResolution::Ambiguous { .. } => {
                panic!("uniquely-running should win");
            }
        }
        let both = [row("a", "running"), row("b", "running")];
        assert!(matches!(
            resolve_active_occupant(&both, None, "seat"),
            OccupantResolution::Ambiguous { .. }
        ));
        let mut map = BTreeMap::new();
        map.insert("seat".to_owned(), Some("newer-exited".to_owned()));
        match resolve_active_occupant(&sessions, Some(&map), "seat") {
            OccupantResolution::Resolved(found) => assert_eq!(found.id, "newer-exited"),
            OccupantResolution::None | OccupantResolution::Ambiguous { .. } => {
                panic!("explicit relation should be consumed");
            }
        }
        map.insert("seat".to_owned(), None);
        assert!(matches!(
            resolve_active_occupant(&sessions, Some(&map), "seat"),
            OccupantResolution::Ambiguous { .. }
        ));
        map.remove("seat");
        assert!(matches!(
            resolve_active_occupant(&sessions, Some(&map), "seat"),
            OccupantResolution::Ambiguous { .. }
        ));
        map.insert("seat".to_owned(), Some("missing-session".to_owned()));
        match resolve_active_occupant(&sessions, Some(&map), "seat") {
            OccupantResolution::Ambiguous { detail, .. } => {
                assert!(detail.contains("missing-session"), "{detail}");
                assert!(detail.contains("dangling"), "{detail}");
            }
            OccupantResolution::Resolved(_) | OccupantResolution::None => {
                panic!("dangling relation should be ambiguous");
            }
        }
    }
}
