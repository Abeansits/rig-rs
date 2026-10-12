//! Poll a Claude pane until it is ready, blocked, or out of budget.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use crewmux_mux::{Capture, CaptureRange, Multiplexer, PaneRef};
use crewmux_types::{ReadinessCode, SeatId};

use crate::budget::LaunchBudget;
use crate::claude::screen::{ClaudeScreen, classify, same_dir, trust_folder};
use crate::dialog::{DialogDriver, DialogOption, MuxDialogDriver, auto_choice};

pub(crate) struct DriveOutcome {
    pub(crate) force: Option<ReadinessCode>,
}

pub(crate) async fn drive(
    mux: Arc<dyn Multiplexer>,
    pane: &PaneRef,
    seat: &SeatId,
    cwd: &Path,
    auto_trust: bool,
    budget: LaunchBudget,
) -> DriveOutcome {
    let mut watch = Watch::new();
    loop {
        if watch.expired(budget) {
            return blocked();
        }
        let Ok(capture) = mux.capture(pane, CaptureRange::Visible).await else {
            return blocked();
        };
        match act(&mux, pane, seat, cwd, auto_trust, &capture).await {
            Act::Ready | Act::Stop => return DriveOutcome { force: None },
            Act::Answered => watch.gate_at = Some(Instant::now()),
            Act::Poll { inconclusive } => {
                if let Some(code) = watch.note(&capture, budget, inconclusive) {
                    return DriveOutcome { force: Some(code) };
                }
            }
        }
        tokio::time::sleep(budget.poll).await;
    }
}

enum Act {
    Ready,
    Stop,
    Answered,
    /// `inconclusive` is false while the pane is busy (`esc to interrupt`).
    Poll {
        inconclusive: bool,
    },
}

async fn act(
    mux: &Arc<dyn Multiplexer>,
    pane: &PaneRef,
    seat: &SeatId,
    cwd: &Path,
    auto_trust: bool,
    capture: &Capture,
) -> Act {
    match classify(capture) {
        ClaudeScreen::Ready(_) => Act::Ready,
        ClaudeScreen::Busy => Act::Poll {
            inconclusive: false,
        },
        ClaudeScreen::Inconclusive => Act::Poll { inconclusive: true },
        ClaudeScreen::Login(_) | ClaudeScreen::UnknownMenu { .. } | ClaudeScreen::Draft => {
            Act::Stop
        }
        ClaudeScreen::Dialog(dialog) => {
            if matches!(dialog.known, crewmux_types::KnownDialog::ClaudeTrust)
                && !trust_folder(&capture.text).is_some_and(|folder| same_dir(&folder, cwd))
            {
                return Act::Stop;
            }
            let Some(choice) = auto_choice(&dialog.known, auto_trust) else {
                return Act::Stop;
            };
            answer(mux, pane, seat, &dialog.known, &choice).await
        }
    }
}

async fn answer(
    mux: &Arc<dyn Multiplexer>,
    pane: &PaneRef,
    seat: &SeatId,
    dialog: &crewmux_types::KnownDialog,
    choice: &DialogOption,
) -> Act {
    let driver = MuxDialogDriver::new(Arc::clone(mux), pane.clone());
    match driver.answer(seat, dialog, choice).await {
        Ok(_) => Act::Answered,
        Err(_) => Act::Stop,
    }
}

fn blocked() -> DriveOutcome {
    DriveOutcome {
        force: Some(ReadinessCode::UnrecognisedScreen),
    }
}

struct Watch {
    started: Instant,
    stable_since: Instant,
    last_hash: Option<u64>,
    gate_at: Option<Instant>,
}

impl Watch {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            started: now,
            stable_since: now,
            last_hash: None,
            gate_at: None,
        }
    }

    fn expired(&self, budget: LaunchBudget) -> bool {
        self.started.elapsed() >= budget.overall
    }

    fn note(
        &mut self,
        capture: &Capture,
        budget: LaunchBudget,
        inconclusive: bool,
    ) -> Option<ReadinessCode> {
        let now = Instant::now();
        if self.last_hash != Some(capture.hash) {
            self.last_hash = Some(capture.hash);
            self.stable_since = now;
        }
        if capture.text.trim().is_empty() && self.started.elapsed() >= budget.first_screen {
            return Some(ReadinessCode::UnrecognisedScreen);
        }
        let stable = now.saturating_duration_since(self.stable_since);
        if inconclusive && !capture.text.trim().is_empty() && stable >= budget.inconclusive_stable {
            return Some(ReadinessCode::UnrecognisedScreen);
        }
        let past_gate = self.gate_at.is_some_and(|gate| {
            now.saturating_duration_since(gate) >= budget.ready_after_last_gate
        });
        if past_gate {
            return Some(ReadinessCode::UnrecognisedScreen);
        }
        None
    }
}
