//! Read-only restore-check report.
//!
//! Confidence classes are `ready`, `ready_with_caveats`, `not_ready`,
//! `attention_required`, and `unknown`. Remediations default to unsafe.

use serde::{Deserialize, Serialize};

/// One confidence class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceClass {
    /// Seat is up and has a resume token.
    Ready,
    /// Seat is up and would restore fresh.
    ReadyWithCaveats,
    /// Seat is dead or stopped.
    NotReady,
    /// A gate or other attention state.
    AttentionRequired,
    /// The probe could not tell. A down daemon is this class.
    Unknown,
}

impl ConfidenceClass {
    /// Wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::ReadyWithCaveats => "ready_with_caveats",
            Self::NotReady => "not_ready",
            Self::AttentionRequired => "attention_required",
            Self::Unknown => "unknown",
        }
    }
}

/// Counts for the five classes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfidenceBreakdown {
    /// [`ConfidenceClass::Ready`].
    pub ready: u32,
    /// [`ConfidenceClass::ReadyWithCaveats`].
    pub ready_with_caveats: u32,
    /// [`ConfidenceClass::NotReady`].
    pub not_ready: u32,
    /// [`ConfidenceClass::AttentionRequired`].
    pub attention_required: u32,
    /// [`ConfidenceClass::Unknown`].
    pub unknown: u32,
}

impl ConfidenceBreakdown {
    fn bump(&mut self, class: ConfidenceClass) {
        let slot = match class {
            ConfidenceClass::Ready => &mut self.ready,
            ConfidenceClass::ReadyWithCaveats => &mut self.ready_with_caveats,
            ConfidenceClass::NotReady => &mut self.not_ready,
            ConfidenceClass::AttentionRequired => &mut self.attention_required,
            ConfidenceClass::Unknown => &mut self.unknown,
        };
        *slot = slot.saturating_add(1);
    }
}

/// One seat in the report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatCheck {
    /// Seat name.
    pub seat: String,
    /// Class.
    pub class: ConfidenceClass,
}

/// A suggested fix. `execution_safe` defaults to false.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remediation {
    /// What an operator could do.
    pub action: String,
    /// False unless the action is a pure read. Mutating actions stay false.
    #[serde(default)]
    pub execution_safe: bool,
}

impl Remediation {
    /// A mutating action. Never execution-safe.
    #[must_use]
    pub fn mutating(action: impl Into<String>) -> Self {
        Self {
            action: action.into(),
            execution_safe: false,
        }
    }
}

/// The restore-check document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreCheckReport {
    /// Class counts.
    pub classes: ConfidenceBreakdown,
    /// Per seat.
    pub seats: Vec<SeatCheck>,
    /// Suggested actions. Each carries `execution_safe`.
    pub remediations: Vec<Remediation>,
}

/// What restore-check could see for one seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeatSignal {
    /// Running, and whether a resume token exists.
    Up {
        /// A resume token was minted for this seat.
        resume: bool,
    },
    /// Dead or stopped.
    Down,
    /// Attention or a gate.
    Attention,
    /// Unclassified.
    Unknown,
}

/// Build a report. A down daemon is `unknown` plus an unsafe start remediation.
#[must_use]
pub fn restore_check(daemon_up: bool, seats: &[(&str, SeatSignal)]) -> RestoreCheckReport {
    if !daemon_up {
        let mut classes = ConfidenceBreakdown::default();
        classes.bump(ConfidenceClass::Unknown);
        return RestoreCheckReport {
            classes,
            seats: Vec::new(),
            remediations: vec![Remediation::mutating("start the daemon")],
        };
    }
    let mut classes = ConfidenceBreakdown::default();
    let mut checked = Vec::new();
    let mut remediations = Vec::new();
    for (name, signal) in seats {
        let class = class_of(*signal);
        classes.bump(class);
        checked.push(SeatCheck {
            seat: (*name).to_owned(),
            class,
        });
        if let Some(action) = remediation_for(*signal) {
            remediations.push(Remediation::mutating(action));
        }
    }
    RestoreCheckReport {
        classes,
        seats: checked,
        remediations,
    }
}

fn class_of(signal: SeatSignal) -> ConfidenceClass {
    match signal {
        SeatSignal::Up { resume: true } => ConfidenceClass::Ready,
        SeatSignal::Up { resume: false } => ConfidenceClass::ReadyWithCaveats,
        SeatSignal::Down => ConfidenceClass::NotReady,
        SeatSignal::Attention => ConfidenceClass::AttentionRequired,
        SeatSignal::Unknown => ConfidenceClass::Unknown,
    }
}

fn remediation_for(signal: SeatSignal) -> Option<&'static str> {
    match signal {
        SeatSignal::Down => Some("relaunch the seat"),
        SeatSignal::Up { resume: false } => Some("mint a resume token"),
        SeatSignal::Attention => Some("clear the gate"),
        SeatSignal::Up { resume: true } | SeatSignal::Unknown => None,
    }
}

/// The mutating actions restore-check may mention. All are unsafe.
#[must_use]
pub fn mutating_actions() -> Vec<Remediation> {
    [
        "start the daemon",
        "chmod the state directory",
        "create files",
        "snapshot",
    ]
    .into_iter()
    .map(Remediation::mutating)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::{ConfidenceClass, SeatSignal, mutating_actions, restore_check};

    #[test]
    fn five_classes_and_unsafe_mutations() {
        let report = restore_check(
            true,
            &[
                ("up", SeatSignal::Up { resume: true }),
                ("fresh", SeatSignal::Up { resume: false }),
                ("dead", SeatSignal::Down),
                ("gate", SeatSignal::Attention),
                ("odd", SeatSignal::Unknown),
            ],
        );
        assert_eq!(report.classes.ready, 1);
        assert_eq!(report.classes.ready_with_caveats, 1);
        assert_eq!(report.classes.not_ready, 1);
        assert_eq!(report.classes.attention_required, 1);
        assert_eq!(report.classes.unknown, 1);
        assert!(report.remediations.iter().all(|item| !item.execution_safe));
        let actions = mutating_actions();
        assert_eq!(actions.len(), 4);
        assert!(actions.iter().all(|item| !item.execution_safe));
        assert_eq!(
            report.seats.first().map(|seat| seat.class),
            Some(ConfidenceClass::Ready)
        );
    }

    #[test]
    fn daemon_down_is_unknown() {
        let report = restore_check(false, &[("one", SeatSignal::Up { resume: true })]);
        assert_eq!(report.classes.unknown, 1);
        assert_eq!(report.seats.len(), 0);
        assert!(
            report
                .remediations
                .iter()
                .any(|item| { item.action.contains("daemon") && !item.execution_safe })
        );
    }
}
