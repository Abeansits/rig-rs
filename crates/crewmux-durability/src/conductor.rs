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

/// The next rig in a crash-cart walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CartNext {
    /// Rig to restore, or to record as not attempted.
    pub rig: RigId,
    /// Kernel rigs are ordered first.
    pub kernel: bool,
    /// False once cancel has been observed after an earlier rig.
    pub run: bool,
}

/// Shared cancel state. [`conduct`] and the daemon both walk this.
///
/// Cancel is recorded after the in-flight rig returns. That rig finishes.
/// Later rigs are [`RigRestoreOutcome::NotAttempted`].
#[derive(Clone, Debug, Default)]
pub struct CartWalk {
    index: usize,
    stop: bool,
    cancelled_before: Option<RigId>,
    rigs: Vec<CartRig>,
}

impl CartWalk {
    /// An empty walk.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The rig at the current index, when one remains.
    #[must_use]
    pub fn next(&self, ordered: &[(RigId, bool)]) -> Option<CartNext> {
        let (rig, kernel) = ordered.get(self.index)?;
        Some(CartNext {
            rig: *rig,
            kernel: *kernel,
            run: !self.stop,
        })
    }

    /// Record `outcome` for the current rig. `cancelled` stops the fleet afterwards.
    pub fn record(
        &mut self,
        ordered: &[(RigId, bool)],
        outcome: RigRestoreOutcome,
        cancelled: bool,
    ) {
        let Some((rig, kernel)) = ordered.get(self.index) else {
            return;
        };
        if self.stop && self.cancelled_before.is_none() {
            self.cancelled_before = Some(*rig);
        }
        self.rigs.push(CartRig {
            rig: *rig,
            outcome,
            kernel: *kernel,
        });
        if cancelled {
            self.stop = true;
        }
        self.index = self.index.saturating_add(1);
    }

    /// The finished run.
    #[must_use]
    pub fn into_run(self) -> CrashCartRun {
        CrashCartRun {
            rigs: self.rigs,
            cancelled_before: self.cancelled_before,
        }
    }
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
    let mut walk = CartWalk::new();
    while let Some(step) = walk.next(ordered) {
        let outcome = if step.run {
            restore(&step.rig)
        } else {
            RigRestoreOutcome::NotAttempted
        };
        let stop = if step.run { cancelled() } else { true };
        walk.record(ordered, outcome, stop);
    }
    walk.into_run()
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
