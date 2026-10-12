//! Move the cursor, re-capture, then Enter.

use std::sync::Arc;

use async_trait::async_trait;
use crewmux_mux::{CaptureRange, Key, Multiplexer, PaneRef};
use crewmux_types::{KnownDialog, SeatId};

use crate::claude::screen::{ClassifiedDialog, identify_dialog};
use crate::screen_common::{cursor_index, menu_lines};

/// The choice the driver should land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DialogOption {
    /// Exact choice text, including a `2.` prefix when the line has one.
    pub text: &'static str,
    /// Number on that line, when the capture showed one.
    pub index: Option<u8>,
}

/// Enter landed on `choice`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answered {
    /// The choice text that was verified.
    pub choice: String,
}

/// The driver stopped before Enter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Escalate {
    /// Why the choice was not submitted.
    pub reason: String,
}

/// Answer one known dialog on a pane.
#[async_trait]
pub trait DialogDriver: Send + Sync {
    /// Move to `choose`, verify the cursor, then send Enter.
    async fn answer(
        &self,
        seat: &SeatId,
        dialog: &KnownDialog,
        choose: &DialogOption,
    ) -> Result<Answered, Escalate>;
}

/// Automatic choice, when this dialog may be answered at all.
#[must_use]
pub fn auto_choice(dialog: &KnownDialog, auto_trust: bool) -> Option<DialogOption> {
    match dialog {
        KnownDialog::ClaudeTrust if auto_trust => Some(DialogOption {
            text: "Yes, I trust this folder",
            index: None,
        }),
        KnownDialog::ClaudeFullscreenRenderer => Some(DialogOption {
            text: "2. Not now",
            index: Some(2),
        }),
        KnownDialog::CodexUpdate => Some(DialogOption {
            text: "2. Skip",
            index: Some(2),
        }),
        KnownDialog::ClaudeTrust
        | KnownDialog::ClaudeBypassConsent
        | KnownDialog::ClaudeNewMcpServer { .. } => None,
    }
}

/// Dialog driver over a [`Multiplexer`].
pub struct MuxDialogDriver {
    mux: Arc<dyn Multiplexer>,
    pane: PaneRef,
}

impl MuxDialogDriver {
    /// `pane` is the seat pane the keys are sent to.
    #[must_use]
    pub fn new(mux: Arc<dyn Multiplexer>, pane: PaneRef) -> Self {
        Self { mux, pane }
    }
}

#[async_trait]
impl DialogDriver for MuxDialogDriver {
    async fn answer(
        &self,
        _seat: &SeatId,
        dialog: &KnownDialog,
        choose: &DialogOption,
    ) -> Result<Answered, Escalate> {
        let current = self.capture().await?;
        let found = require_dialog(&current.text, dialog)?;
        let target = option_at(&found, choose.text)?;
        let cursor = cursor_of(&found)?;
        self.move_cursor(cursor, target).await?;
        let verified = self.capture().await?;
        let after = require_dialog(&verified.text, dialog)?;
        if cursor_text(&after) != Some(choose.text) {
            return Err(Escalate {
                reason: "cursor was not on the requested choice".to_owned(),
            });
        }
        self.mux
            .send_keys(&self.pane, &[Key::Enter])
            .await
            .map_err(|err| Escalate {
                reason: err.to_string(),
            })?;
        self.expect_dialog_gone(dialog).await?;
        Ok(Answered {
            choice: choose.text.to_owned(),
        })
    }
}

impl MuxDialogDriver {
    async fn capture(&self) -> Result<crewmux_mux::Capture, Escalate> {
        self.mux
            .capture(&self.pane, CaptureRange::Visible)
            .await
            .map_err(|err| Escalate {
                reason: err.to_string(),
            })
    }

    async fn move_cursor(&self, cursor: usize, target: usize) -> Result<(), Escalate> {
        let steps = cursor.abs_diff(target);
        if steps == 0 {
            return Ok(());
        }
        let down = target > cursor;
        for _ in 0..steps {
            let key = if down { Key::Down } else { Key::Up };
            self.mux
                .send_keys(&self.pane, &[key])
                .await
                .map_err(|err| Escalate {
                    reason: err.to_string(),
                })?;
        }
        Ok(())
    }

    async fn expect_dialog_gone(&self, dialog: &KnownDialog) -> Result<(), Escalate> {
        let seen = self.capture().await?;
        let still = identify_dialog(&seen.text, &menu_lines(&seen.text));
        if still.is_some_and(|found| found.known == *dialog) {
            return Err(Escalate {
                reason: "dialog still on screen after enter".to_owned(),
            });
        }
        Ok(())
    }
}

fn require_dialog(text: &str, expected: &KnownDialog) -> Result<ClassifiedDialog, Escalate> {
    let found = identify_dialog(text, &menu_lines(text)).ok_or_else(|| Escalate {
        reason: "screen is not a known dialog".to_owned(),
    })?;
    if found.known != *expected {
        return Err(Escalate {
            reason: "screen is a different dialog".to_owned(),
        });
    }
    Ok(found)
}

fn option_at(dialog: &ClassifiedDialog, text: &str) -> Result<usize, Escalate> {
    dialog
        .options
        .iter()
        .position(|line| line.text == text)
        .ok_or_else(|| Escalate {
            reason: format!("choice {text} is not on screen"),
        })
}

fn cursor_of(dialog: &ClassifiedDialog) -> Result<usize, Escalate> {
    cursor_index(&dialog.options).ok_or_else(|| Escalate {
        reason: "dialog has no cursor".to_owned(),
    })
}

fn cursor_text(dialog: &ClassifiedDialog) -> Option<&str> {
    let index = cursor_index(&dialog.options)?;
    dialog.options.get(index).map(|line| line.text.as_str())
}
