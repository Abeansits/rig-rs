//! Move one step, re-capture, and Enter only with a [`VerifiedCursor`].
//!
//! A live dialog waits until the capture is stable, pauses after each arrow,
//! and requires the target on two captures before Enter. `No, exit` is never
//! confirmed. A short launch budget skips the wall wait; the two-capture rule
//! still applies.

use std::sync::Arc;

use async_trait::async_trait;
use crewmux_mux::{CaptureRange, Key, Multiplexer, PaneRef};
use crewmux_types::DialogId;

use crate::dialog::pace::{DialogClock, DialogPace, WallClock};
use crate::dialog::policy::{AnswerPolicy, EscalateReason, Plan, SeatContext, never_choose, plan};
use crate::dialog::recognise::{Recognised, recognise};
use crate::dialog::table::DialogSpec;
use crate::error::Error;

/// The only keys the driver uses to move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavKey {
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
}

/// Proof that the latest capture has the cursor on `label`.
///
/// Only [`DialogDriver::drive`] can construct this. [`PaneIo::enter`] requires it.
#[expect(
    clippy::manual_non_exhaustive,
    reason = "a private field keeps construction inside this module; non_exhaustive would allow other modules in the crate"
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedCursor {
    /// Line index of the selected choice.
    pub line: usize,
    /// Label the cursor was on.
    pub label: String,
    _priv: (),
}

/// Pane operations the driver is allowed to perform.
#[async_trait]
pub trait PaneIo: Send + Sync {
    /// Visible text.
    async fn capture_text(&self) -> Result<String, Error>;

    /// Move the cursor one row.
    async fn nav(&self, key: NavKey) -> Result<(), Error>;

    /// Send Enter. Impossible without a [`VerifiedCursor`].
    async fn enter(&self, proof: VerifiedCursor) -> Result<(), Error>;
}

/// [`PaneIo`] over a [`Multiplexer`].
pub struct MuxPane {
    mux: Arc<dyn Multiplexer>,
    pane: PaneRef,
}

impl MuxPane {
    /// `pane` receives the keys.
    #[must_use]
    pub fn new(mux: Arc<dyn Multiplexer>, pane: PaneRef) -> Self {
        Self { mux, pane }
    }
}

#[async_trait]
impl PaneIo for MuxPane {
    async fn capture_text(&self) -> Result<String, Error> {
        Ok(self
            .mux
            .capture(&self.pane, CaptureRange::Visible)
            .await?
            .text)
    }

    async fn nav(&self, key: NavKey) -> Result<(), Error> {
        let key = match key {
            NavKey::Up => Key::Up,
            NavKey::Down => Key::Down,
        };
        self.mux.send_keys(&self.pane, &[key]).await?;
        Ok(())
    }

    async fn enter(&self, proof: VerifiedCursor) -> Result<(), Error> {
        if never_choose(&proof.label) {
            return Err(Error::Message(
                "refusing Enter on a deny-listed choice".to_owned(),
            ));
        }
        self.mux.send_keys(&self.pane, &[Key::Enter]).await?;
        Ok(())
    }
}

/// How many navigation steps [`DialogDriver::drive`] will take.
pub struct DialogDriver {
    /// Step budget. The default is 8.
    pub max_steps: u8,
    pace: DialogPace,
    clock: Arc<dyn DialogClock>,
}

impl Default for DialogDriver {
    fn default() -> Self {
        Self::paced(DialogPace::immediate(), Arc::new(WallClock::new()))
    }
}

impl DialogDriver {
    /// Live timings: 1s of stability, then 400ms after each arrow and before Enter.
    #[must_use]
    pub fn live() -> Self {
        Self::paced(DialogPace::live(), Arc::new(WallClock::new()))
    }

    /// `pace` and `clock`. A [`crate::dialog::ManualClock`] lets tests skip the wall wait.
    #[must_use]
    pub fn paced(pace: DialogPace, clock: Arc<dyn DialogClock>) -> Self {
        Self {
            max_steps: 8,
            pace,
            clock,
        }
    }
}

/// Result of one drive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DriveOutcome {
    /// Enter landed and the dialog left the screen.
    Answered {
        /// Dialog that was answered.
        id: DialogId,
        /// Label that was verified.
        label: &'static str,
    },
    /// No Enter was sent, or Enter did not dismiss the dialog.
    Escalated {
        /// Dialog id when one had been recognised.
        id: Option<DialogId>,
        /// Why the driver stopped.
        reason: EscalateReason,
    },
    /// The capture matched no row.
    NotADialog,
}

impl DialogDriver {
    /// Capture, recognise, plan, then step until the cursor is verified.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when capture or a key send fails.
    pub async fn drive(
        &self,
        io: &dyn PaneIo,
        table: &'static [DialogSpec],
        policy: AnswerPolicy,
        seat: &SeatContext<'_>,
    ) -> Result<DriveOutcome, Error> {
        let text = io.capture_text().await?;
        let Some(found) = recognise(table, &text) else {
            return Ok(DriveOutcome::NotADialog);
        };
        let id = found.spec.id;
        // Policy runs before any wait or key. `auto_trust: false` sends nothing.
        if let Plan::Escalate(reason) = plan(&found, policy, seat) {
            return Ok(DriveOutcome::Escalated {
                id: Some(id),
                reason,
            });
        }
        let mut current = match self.settle(io, table, id, text).await? {
            Ok(found) => found,
            Err(reason) => {
                return Ok(DriveOutcome::Escalated {
                    id: Some(id),
                    reason,
                });
            }
        };
        let mut steps = 0u8;
        while steps < self.max_steps {
            let Plan::Answer(label) = plan(&current, policy, seat) else {
                return Ok(DriveOutcome::Escalated {
                    id: Some(id),
                    reason: EscalateReason::Policy,
                });
            };
            match self.confirm(io, table, id, label, &current).await? {
                Confirm::Enter(proof) => {
                    return self.finish_enter(io, table, id, label, proof).await;
                }
                Confirm::Unmoved => {}
                Confirm::Moved(again) => {
                    current = again;
                    steps = steps.saturating_add(1);
                    continue;
                }
                Confirm::Stop(reason) => {
                    return Ok(DriveOutcome::Escalated {
                        id: Some(id),
                        reason,
                    });
                }
            }
            match self.nudge(io, table, id, label, &current).await? {
                Nudge::Moved(again) => current = again,
                Nudge::Stop(reason) => {
                    return Ok(DriveOutcome::Escalated {
                        id: Some(id),
                        reason,
                    });
                }
            }
            steps = steps.saturating_add(1);
        }
        Ok(DriveOutcome::Escalated {
            id: Some(id),
            reason: EscalateReason::StepLimit,
        })
    }

    async fn finish_enter(
        &self,
        io: &dyn PaneIo,
        table: &'static [DialogSpec],
        id: DialogId,
        label: &'static str,
        proof: VerifiedCursor,
    ) -> Result<DriveOutcome, Error> {
        io.enter(proof).await?;
        let after = io.capture_text().await?;
        if recognise(table, &after).is_some_and(|again| again.spec.id == id) {
            return Ok(DriveOutcome::Escalated {
                id: Some(id),
                reason: EscalateReason::DialogRemained,
            });
        }
        Ok(DriveOutcome::Answered { id, label })
    }

    async fn nudge(
        &self,
        io: &dyn PaneIo,
        table: &'static [DialogSpec],
        id: DialogId,
        label: &str,
        current: &Recognised,
    ) -> Result<Nudge, Error> {
        let room = current.menu.len().saturating_sub(1);
        if room == 0 {
            return Ok(Nudge::Stop(EscalateReason::CursorMismatch));
        }
        let Some(key) = step_toward(current, label) else {
            return Ok(Nudge::Stop(EscalateReason::CursorMismatch));
        };
        io.nav(key).await?;
        self.clock.wait(self.pace.nav_settle).await;
        match self.read_dialog(io, table, id).await? {
            Some(again) => Ok(Nudge::Moved(again)),
            None => Ok(Nudge::Stop(EscalateReason::Unrecognised)),
        }
    }

    /// Unchanged for [`DialogPace::stable`].
    async fn settle(
        &self,
        io: &dyn PaneIo,
        table: &'static [DialogSpec],
        id: DialogId,
        first: String,
    ) -> Result<Result<Recognised, EscalateReason>, Error> {
        if self.pace.stable.is_zero() {
            return Ok(same_dialog(table, id, &first).ok_or(EscalateReason::Unrecognised));
        }
        let mut seen = first;
        let mut since = self.clock.elapsed();
        let origin = since;
        let budget = self.pace.stable.saturating_mul(4);
        loop {
            let elapsed = self.clock.elapsed();
            if elapsed.saturating_sub(since) >= self.pace.stable {
                return Ok(same_dialog(table, id, &seen).ok_or(EscalateReason::Unrecognised));
            }
            if elapsed.saturating_sub(origin) >= budget {
                return Ok(Err(EscalateReason::StepLimit));
            }
            let slice = self
                .pace
                .stable
                .saturating_sub(elapsed.saturating_sub(since));
            self.clock.wait(slice).await;
            let next = io.capture_text().await?;
            if next != seen {
                seen = next;
                since = self.clock.elapsed();
            }
        }
    }

    /// Target on this capture and on a later one. A redraw between them is not Enter.
    async fn confirm(
        &self,
        io: &dyn PaneIo,
        table: &'static [DialogSpec],
        id: DialogId,
        label: &str,
        current: &Recognised,
    ) -> Result<Confirm, Error> {
        if verified(current, label).is_none() {
            return Ok(Confirm::Unmoved);
        }
        self.clock.wait(self.pace.confirm_gap).await;
        let Some(again) = self.read_dialog(io, table, id).await? else {
            return Ok(Confirm::Stop(EscalateReason::Unrecognised));
        };
        if let Some(proof) = verified(&again, label) {
            Ok(Confirm::Enter(proof))
        } else {
            Ok(Confirm::Moved(again))
        }
    }

    async fn read_dialog(
        &self,
        io: &dyn PaneIo,
        table: &'static [DialogSpec],
        id: DialogId,
    ) -> Result<Option<Recognised>, Error> {
        let text = io.capture_text().await?;
        Ok(same_dialog(table, id, &text))
    }
}

fn same_dialog(table: &'static [DialogSpec], id: DialogId, text: &str) -> Option<Recognised> {
    recognise(table, text).filter(|found| found.spec.id == id)
}

enum Nudge {
    Moved(Recognised),
    Stop(EscalateReason),
}

enum Confirm {
    Enter(VerifiedCursor),
    /// The cursor is not on the target. The caller still holds the recognition.
    Unmoved,
    /// The confirm capture moved off the target.
    Moved(Recognised),
    Stop(EscalateReason),
}

fn verified(found: &Recognised, label: &str) -> Option<VerifiedCursor> {
    let option = found.menu.iter().find(|option| option.selected)?;
    if option.label != label || never_choose(&option.label) {
        return None;
    }
    Some(VerifiedCursor {
        line: option.line,
        label: option.label.clone(),
        _priv: (),
    })
}

fn step_toward(found: &Recognised, label: &str) -> Option<NavKey> {
    let cursor = found.menu.iter().position(|option| option.selected)?;
    let target = found.menu.iter().position(|option| option.label == label)?;
    match target.cmp(&cursor) {
        std::cmp::Ordering::Greater => Some(NavKey::Down),
        std::cmp::Ordering::Less => Some(NavKey::Up),
        std::cmp::Ordering::Equal => None,
    }
}
