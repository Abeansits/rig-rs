//! Move the cursor, re-capture, then Enter.

use async_trait::async_trait;
use crewmux_types::Key;

use crate::dialog::known::DialogChoice;
use crate::dialog::options::menu_options;
use crate::error::Error;

/// A pane the driver can read and send keys to.
#[async_trait]
pub trait PaneIo: Send + Sync {
    /// Visible text.
    async fn capture_text(&self) -> Result<String, Error>;

    /// Deliver keys. The driver only sends Up, Down, and Enter.
    async fn send_keys(&self, keys: &[Key]) -> Result<(), Error>;
}

/// Outcome of one answer attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DriveResult {
    /// The dialog left the screen after a verified Enter.
    Answered,
    /// The driver stopped. Enter was not sent unless the dialog failed to close.
    Escalated {
        /// Why the answer stopped.
        reason: String,
    },
}

/// Cursor-verified dialog answers.
#[derive(Debug, Default)]
pub struct DialogDriver;

impl DialogDriver {
    /// Move onto `choice` and press Enter only after the cursor is on that label.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when capture or `send_keys` fails.
    pub async fn answer(io: &dyn PaneIo, choice: DialogChoice) -> Result<DriveResult, Error> {
        let target = choice.label();
        if forbidden(target) {
            return Ok(stop("refusing a destructive option"));
        }
        let mut steps = 0u32;
        loop {
            let text = io.capture_text().await?;
            let options = menu_options(&text);
            if steps > u32::try_from(options.len()).unwrap_or(8) {
                return Ok(stop("cursor did not reach the option"));
            }
            let Some(step) = next_step(&options, target) else {
                return Ok(stop("dialog options changed"));
            };
            match step {
                Step::Confirm => return confirm(io, target).await,
                Step::Move(key) => {
                    io.send_keys(&[key]).await?;
                    steps = steps.saturating_add(1);
                }
                Step::Stop(reason) => return Ok(stop(reason)),
            }
        }
    }
}

enum Step {
    Move(Key),
    Confirm,
    Stop(&'static str),
}

fn next_step(options: &[crate::dialog::options::MenuOption], target: &str) -> Option<Step> {
    let current = options.iter().find(|opt| opt.selected)?;
    if current.label == target {
        return Some(Step::Confirm);
    }
    let want = options.iter().position(|opt| opt.label == target)?;
    let have = options.iter().position(|opt| opt.selected)?;
    if have == want {
        return Some(Step::Stop("cursor label does not match the target"));
    }
    let key = if have < want { Key::Down } else { Key::Up };
    Some(Step::Move(key))
}

async fn confirm(io: &dyn PaneIo, target: &str) -> Result<DriveResult, Error> {
    let text = io.capture_text().await?;
    let options = menu_options(&text);
    let Some(current) = options.iter().find(|opt| opt.selected) else {
        return Ok(stop("cursor missing before enter"));
    };
    if current.label != target || forbidden(&current.label) {
        return Ok(stop("cursor is not on the verified option"));
    }
    io.send_keys(&[Key::Enter]).await?;
    let after = io.capture_text().await?;
    if menu_options(&after).iter().any(|opt| opt.label == target) {
        return Ok(stop("dialog still open after enter"));
    }
    Ok(DriveResult::Answered)
}

fn stop(reason: &str) -> DriveResult {
    DriveResult::Escalated {
        reason: reason.to_owned(),
    }
}

fn forbidden(label: &str) -> bool {
    label.contains("Update now")
        || label == "Allow"
        || label.contains("Always allow")
        || label == "Cancel"
        || label.contains("No, exit")
        || label.contains("Run without daemon")
}
