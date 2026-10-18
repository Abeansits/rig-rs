//! Poll a Claude pane until it is ready, blocked, or out of budget.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use crewmux_mux::{Capture, CaptureRange, Multiplexer, PaneRef};
use crewmux_types::ReadinessCode;

use crate::budget::LaunchBudget;
use crate::claude::dialogs::CLAUDE_DIALOGS;
use crate::claude::screen::{ClaudeScreen, classify};
use crate::dialog::{
    AnswerPolicy, DialogDriver, DriveOutcome as DialogDrive, MuxPane, SeatContext,
};

pub(crate) struct DriveOutcome {
    pub(crate) force: Option<ReadinessCode>,
}

pub(crate) async fn drive(
    mux: Arc<dyn Multiplexer>,
    pane: &PaneRef,
    cwd: &Path,
    home: &Path,
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
        let ctx = AnswerCtx {
            cwd,
            home,
            auto_trust,
            budget,
        };
        match act(&mux, pane, &ctx, &capture).await {
            Act::Ready | Act::Stop => return DriveOutcome { force: None },
            Act::Answered => watch.gate_at = Some(Instant::now()),
            Act::Poll {
                inconclusive,
                interrupt,
            } => {
                if let Some(code) = watch.note(&capture, budget, inconclusive, interrupt) {
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
    /// `interrupt` is an `esc to interrupt` row in the live region and no spinner.
    /// That wait uses [`LaunchBudget::inconclusive_stable`] and ignores scrollback.
    Poll {
        inconclusive: bool,
        interrupt: bool,
    },
}

struct AnswerCtx<'a> {
    cwd: &'a Path,
    home: &'a Path,
    auto_trust: bool,
    budget: LaunchBudget,
}

async fn act(
    mux: &Arc<dyn Multiplexer>,
    pane: &PaneRef,
    ctx: &AnswerCtx<'_>,
    capture: &Capture,
) -> Act {
    match classify(capture) {
        ClaudeScreen::Ready(_) => Act::Ready,
        ClaudeScreen::Busy => Act::Poll {
            inconclusive: false,
            interrupt: crate::claude::screen::bounded_interrupt(&capture.text),
        },
        ClaudeScreen::Inconclusive => Act::Poll {
            inconclusive: true,
            interrupt: false,
        },
        ClaudeScreen::Login(_) | ClaudeScreen::UnknownMenu { .. } | ClaudeScreen::Draft => {
            Act::Stop
        }
        ClaudeScreen::Dialog(_) => answer(mux, pane, ctx).await,
    }
}

async fn answer(mux: &Arc<dyn Multiplexer>, pane: &PaneRef, ctx: &AnswerCtx<'_>) -> Act {
    let io = MuxPane::new(Arc::clone(mux), pane.clone());
    let policy = AnswerPolicy {
        auto_trust: ctx.auto_trust,
        auto_approve_rig_tools: false,
    };
    let seat = SeatContext {
        cwd: ctx.cwd,
        home: ctx.home,
    };
    match dialog_driver(ctx.budget)
        .drive(&io, CLAUDE_DIALOGS, policy, &seat)
        .await
    {
        Ok(DialogDrive::Answered { .. }) => Act::Answered,
        Ok(DialogDrive::Escalated { .. } | DialogDrive::NotADialog) | Err(_) => Act::Stop,
    }
}

/// A launch budget under 5s is the test budget. Wall waits would spend it.
/// The two-capture rule still runs. A live launch uses the 1s / 400ms pace.
fn dialog_driver(budget: LaunchBudget) -> DialogDriver {
    if budget.overall < std::time::Duration::from_secs(5) {
        DialogDriver::default()
    } else {
        DialogDriver::live()
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
        interrupt: bool,
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
        let stalled = !capture.text.trim().is_empty() && stable >= budget.inconclusive_stable;
        if stalled && (inconclusive || interrupt) {
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

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use crewmux_mux::{Capture, FakeMux, PaneRef};
    use crewmux_types::ReadinessCode;

    use super::drive;
    use crate::budget::LaunchBudget;

    #[tokio::test]
    async fn interrupt_wait_uses_the_inconclusive_budget() {
        let mux = Arc::new(FakeMux::new());
        mux.set_capture(Capture::new("esc to interrupt\n".to_owned(), (0, 0)))
            .await;
        let budget = LaunchBudget {
            first_screen: Duration::from_secs(5),
            ready_after_last_gate: Duration::from_secs(5),
            inconclusive_stable: Duration::from_millis(40),
            overall: Duration::from_secs(5),
            poll: Duration::from_millis(5),
        };
        let started = Instant::now();
        let outcome = drive(
            mux,
            &PaneRef::session("seat"),
            Path::new("/work"),
            Path::new("/home"),
            false,
            budget,
        )
        .await;
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(outcome.force, Some(ReadinessCode::UnrecognisedScreen));
    }
}
