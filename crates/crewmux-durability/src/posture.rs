//! Restore never widens a seat's posture.

use crewmux_types::LaunchPosture;

/// Rank. Lower is stricter.
fn rank(posture: LaunchPosture) -> u8 {
    match posture {
        LaunchPosture::Locked => 0,
        LaunchPosture::Floor => 1,
        LaunchPosture::Auto => 2,
        LaunchPosture::FullBypass => 3,
    }
}

/// The stricter of the recorded posture and the one a caller asked for.
#[must_use]
pub fn clamp_posture(recorded: LaunchPosture, requested: LaunchPosture) -> LaunchPosture {
    if rank(recorded) <= rank(requested) {
        recorded
    } else {
        requested
    }
}

/// Wire token for a posture.
#[must_use]
pub const fn posture_token(posture: LaunchPosture) -> &'static str {
    match posture {
        LaunchPosture::Locked => "locked",
        LaunchPosture::Floor => "floor",
        LaunchPosture::Auto => "auto",
        LaunchPosture::FullBypass => "full_bypass",
    }
}

/// Parse a stored posture token. Unknown tokens stay at the locked floor.
#[must_use]
pub fn parse_posture(text: &str) -> LaunchPosture {
    match text {
        "locked" => LaunchPosture::Locked,
        "auto" => LaunchPosture::Auto,
        "full_bypass" => LaunchPosture::FullBypass,
        // `floor` and any unknown token stay at the floor.
        _ => LaunchPosture::Floor,
    }
}

#[cfg(test)]
mod tests {
    use super::clamp_posture;
    use crewmux_types::LaunchPosture;

    #[test]
    fn locked_restore_never_permissive() {
        let locked = clamp_posture(LaunchPosture::Locked, LaunchPosture::FullBypass);
        let floor = clamp_posture(LaunchPosture::Floor, LaunchPosture::Auto);
        assert_eq!(locked, LaunchPosture::Locked);
        assert_eq!(floor, LaunchPosture::Floor);
        assert_eq!(
            clamp_posture(LaunchPosture::FullBypass, LaunchPosture::Floor),
            LaunchPosture::Floor
        );
    }
}
