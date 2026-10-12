// Derived from openrig packages/daemon/src/domain/crash-cart-conductor.ts (v0.6.9, Apache-2.0); ported to Rust and modified.

//! Fleet restore. Kernel rigs go first. Cancel stops before the next rig.
//! One failure does not stop the others.

use crewmux_types::RigId;

use crate::outcome::RigRestoreOutcome;

/// One crash-cart pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrashCartRun {
    /// Rig, outcome, and whether it was a kernel rig.
    pub rigs: Vec<CartRig>,
    /// The first rig that was not started because of cancel.
    pub cancelled_before: Option<RigId>,
}

/// One rig in the run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CartRig {
    /// Rig id.
    pub rig: RigId,
    /// Outcome.
    pub outcome: RigRestoreOutcome,
    /// Kernel rigs run before the others.
    pub kernel: bool,
}

/// Kernel rigs first, preserving the caller's order inside each group.
#[must_use]
pub fn kernel_first(rigs: &[(RigId, bool)]) -> Vec<(RigId, bool)> {
    let mut kernel = Vec::new();
    let mut rest = Vec::new();
    for (id, is_kernel) in rigs {
        if *is_kernel {
            kernel.push((*id, true));
        } else {
            rest.push((*id, false));
        }
    }
    kernel.append(&mut rest);
    kernel
}

/// Restore `ordered` one at a time.
///
/// `cancelled` is polled after a rig finishes. The in-flight rig completes.
/// Later rigs are [`RigRestoreOutcome::NotAttempted`].
pub fn conduct(
    ordered: &[(RigId, bool)],
    mut restore: impl FnMut(&RigId) -> RigRestoreOutcome,
    mut cancelled: impl FnMut() -> bool,
) -> CrashCartRun {
    let mut rigs = Vec::new();
    let mut cancelled_before = None;
    let mut stop = false;
    for (rig, kernel) in ordered {
        if stop {
            if cancelled_before.is_none() {
                cancelled_before = Some(*rig);
            }
            rigs.push(CartRig {
                rig: *rig,
                outcome: RigRestoreOutcome::NotAttempted,
                kernel: *kernel,
            });
            continue;
        }
        rigs.push(CartRig {
            rig: *rig,
            outcome: restore(rig),
            kernel: *kernel,
        });
        if cancelled() {
            stop = true;
        }
    }
    CrashCartRun {
        rigs,
        cancelled_before,
    }
}

#[cfg(test)]
mod tests {
    use super::{conduct, kernel_first};
    use crate::outcome::RigRestoreOutcome;
    use crewmux_types::RigId;

    #[test]
    fn restores_rigs_kernel_first_in_order() {
        let kernel = RigId::new();
        let other = RigId::new();
        let third = RigId::new();
        let ordered = kernel_first(&[(other, false), (kernel, true), (third, false)]);
        let ids: Vec<_> = ordered.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec![kernel, other, third]);
    }

    #[test]
    fn one_failure_never_halts_the_fleet() {
        let first = RigId::new();
        let second = RigId::new();
        let run = conduct(
            &[(first, true), (second, false)],
            |rig| {
                if rig == &first {
                    RigRestoreOutcome::Failed
                } else {
                    RigRestoreOutcome::FullyRestored
                }
            },
            || false,
        );
        assert_eq!(
            run.rigs.first().map(|rig| rig.outcome),
            Some(RigRestoreOutcome::Failed)
        );
        assert_eq!(
            run.rigs.get(1).map(|rig| rig.outcome),
            Some(RigRestoreOutcome::FullyRestored)
        );
        assert!(run.cancelled_before.is_none());
    }

    #[test]
    fn cancel_lets_the_in_flight_rig_finish() {
        let first = RigId::new();
        let second = RigId::new();
        let third = RigId::new();
        let calls = std::sync::atomic::AtomicU32::new(0);
        let run = conduct(
            &[(first, true), (second, false), (third, false)],
            |_| {
                calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                RigRestoreOutcome::FullyRestored
            },
            || calls.load(std::sync::atomic::Ordering::Relaxed) >= 1,
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(
            run.rigs.first().map(|rig| rig.outcome),
            Some(RigRestoreOutcome::FullyRestored)
        );
        assert_eq!(
            run.rigs.get(1).map(|rig| rig.outcome),
            Some(RigRestoreOutcome::NotAttempted)
        );
        assert_eq!(
            run.rigs.get(2).map(|rig| rig.outcome),
            Some(RigRestoreOutcome::NotAttempted)
        );
        assert_eq!(run.cancelled_before.as_ref(), Some(&second));
    }
}
