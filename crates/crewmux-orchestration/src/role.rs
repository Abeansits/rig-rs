// Derived from openrig packages/daemon/src/domain/workflow-role-resolver.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Pure role-to-seat selection. Input order never changes the winner.

/// Facts the selector is allowed to see.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeatCandidate {
    /// Stable logical id.
    pub logical_id: String,
    /// Declared role. Empty when the seat has none.
    pub role: Option<String>,
    /// `agent` or `infrastructure`.
    pub node_kind: String,
    /// Lifecycle token. Only `running` qualifies.
    pub lifecycle_state: String,
    /// Agent runtime, when known.
    pub runtime: Option<String>,
    /// Pending items. Least backlog wins.
    pub pending_work_count: i64,
    /// Canonical `{pod}-{member}@{rig}`.
    pub coordinate: Option<String>,
    /// Occupant session name. Adopted seats differ from `coordinate`.
    pub raw_session_name: Option<String>,
}

/// One disqualified seat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disqualified {
    /// Coordinate, when one exists.
    pub coordinate: Option<String>,
    /// Logical id.
    pub logical_id: String,
    /// Named reason.
    pub disqualifier: String,
}

/// The ranked result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RolePick {
    /// Winning coordinate.
    pub seat: Option<String>,
    /// Seats that did not qualify.
    pub disqualified: Vec<Disqualified>,
    /// Qualified coordinates, winner first.
    pub qualified: Vec<String>,
}

/// Select a seat for `role`. Infrastructure nodes are omitted.
#[must_use]
pub fn select_role_seat(
    role: &str,
    harness: Option<&str>,
    candidates: &[SeatCandidate],
) -> RolePick {
    let mut disqualified = Vec::new();
    let mut qualified = Vec::new();
    for candidate in candidates {
        if candidate.node_kind != "agent" {
            continue;
        }
        if let Some(reason) = disqualifier(role, harness, candidate) {
            disqualified.push(Disqualified {
                coordinate: candidate.coordinate.clone(),
                logical_id: candidate.logical_id.clone(),
                disqualifier: reason,
            });
        } else {
            qualified.push(candidate);
        }
    }
    qualified.sort_by(|left, right| {
        left.pending_work_count
            .cmp(&right.pending_work_count)
            .then_with(|| {
                let left_key = left.coordinate.as_deref().unwrap_or("");
                let right_key = right.coordinate.as_deref().unwrap_or("");
                codepoint_cmp(left_key, right_key)
            })
    });
    RolePick {
        seat: qualified
            .first()
            .and_then(|candidate| candidate.coordinate.clone()),
        disqualified,
        qualified: qualified
            .iter()
            .filter_map(|candidate| candidate.coordinate.clone())
            .collect(),
    }
}

fn disqualifier(role: &str, harness: Option<&str>, candidate: &SeatCandidate) -> Option<String> {
    if candidate.role.as_deref() != Some(role) {
        return Some("role_not_declared".to_owned());
    }
    if candidate.lifecycle_state != "running" {
        return Some(format!(
            "not_live(lifecycleState={})",
            candidate.lifecycle_state
        ));
    }
    if candidate.coordinate.is_none() {
        return Some("coordinate_underivable".to_owned());
    }
    if candidate.raw_session_name.is_some()
        && candidate.raw_session_name != candidate.coordinate
    {
        return Some("adopted_seat_not_role_resolvable_v1".to_owned());
    }
    if let Some(harness) = harness
        && candidate.runtime.as_deref() != Some(harness)
    {
        let actual = candidate.runtime.as_deref().unwrap_or("unknown");
        return Some(format!("runtime_mismatch({actual}≠{harness})"));
    }
    None
}

fn codepoint_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    let mut left_chars = left.chars();
    let mut right_chars = right.chars();
    loop {
        match (left_chars.next(), right_chars.next()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(left_ch), Some(right_ch)) => {
                let order = u32::from(left_ch).cmp(&u32::from(right_ch));
                if order != std::cmp::Ordering::Equal {
                    return order;
                }
            }
        }
    }
}
