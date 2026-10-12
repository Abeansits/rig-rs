//! Move one step, re-capture, and Enter only with a [`VerifiedCursor`].

use std::sync::Arc;

use async_trait::async_trait;
use crewmux_mux::{CaptureRange, Key, Multiplexer, PaneRef};
use crewmux_types::DialogId;

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
}

impl Default for DialogDriver {
    fn default() -> Self {
        Self { max_steps: 8 }
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
        let label = match plan(&found, policy, seat) {
            Plan::Answer(label) => label,
            Plan::Escalate(reason) => {
                return Ok(DriveOutcome::Escalated {
                    id: Some(id),
                    reason,
                });
            }
        };
        let mut current = found;
        for _ in 0..self.max_steps {
            if let Some(proof) = verified(&current, label) {
                io.enter(proof).await?;
                let after = io.capture_text().await?;
                if recognise(table, &after).is_some_and(|again| again.spec.id == id) {
                    return Ok(DriveOutcome::Escalated {
                        id: Some(id),
                        reason: EscalateReason::DialogRemained,
                    });
                }
                return Ok(DriveOutcome::Answered { id, label });
            }
            let Some(key) = step_toward(&current, label) else {
                return Ok(DriveOutcome::Escalated {
                    id: Some(id),
                    reason: EscalateReason::CursorMismatch,
                });
            };
            io.nav(key).await?;
            let next = io.capture_text().await?;
            let Some(again) = recognise(table, &next) else {
                return Ok(DriveOutcome::Escalated {
                    id: Some(id),
                    reason: EscalateReason::Unrecognised,
                });
            };
            if again.spec.id != id {
                return Ok(DriveOutcome::Escalated {
                    id: Some(id),
                    reason: EscalateReason::Unrecognised,
                });
            }
            current = again;
        }
        Ok(DriveOutcome::Escalated {
            id: Some(id),
            reason: EscalateReason::StepLimit,
        })
    }
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
