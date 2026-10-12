//! Submit only while the composer is ready, then require a byte-exact rollout item.

use std::time::Instant;

use crewmux_mux::PasteOpts;
use crewmux_types::{Key, ReadinessCode, SeatId, ThreadId};

use crate::codex::adapter::{CodexAdapter, IdQuery, blocking};
use crate::codex::rollout::{ThreadIdError, user_turn_texts};
use crate::codex::screen::CodexScreen;
use crate::error::Error;
use crate::types::{DeliveryReceipt, NodeBinding, SubmitMode};

impl CodexAdapter {
    /// Paste `text` and, in [`SubmitMode::Enter`], send Enter.
    ///
    /// Refuses every screen that is not [`CodexScreen::Ready`]. The rollout's
    /// user item must equal `text` byte for byte.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the screen is not ready, the pane write fails, or
    /// the rollout does not contain `text` before the submit budget ends.
    pub async fn submit(
        &self,
        binding: &NodeBinding,
        text: &str,
        mode: SubmitMode,
    ) -> Result<DeliveryReceipt, Error> {
        let screen = self.visible_screen(binding).await?;
        if !matches!(screen, CodexScreen::Ready(_)) {
            return Err(Error::Message(
                "submit requires a ready composer".to_owned(),
            ));
        }
        self.mux()
            .paste(&binding.pane, text, PasteOpts::default())
            .await?;
        if mode == SubmitMode::Enter {
            let pasted = self.visible_screen(binding).await?;
            if !draft_is_prefix(&pasted, text) {
                return Err(Error::Message("composer did not show the paste".to_owned()));
            }
            self.mux().send_keys(&binding.pane, &[Key::Enter]).await?;
        }
        self.confirm_delivery(binding.seat, text).await?;
        Ok(DeliveryReceipt { bytes: text.len() })
    }

    /// Interrupt is not a pane key. Stop uses the tmux kill path.
    ///
    /// # Errors
    ///
    /// Always returns an error. No key is sent.
    #[expect(
        clippy::unused_self,
        reason = "interrupt must not send keys into the pane"
    )]
    pub fn interrupt(&self, _binding: &NodeBinding) -> Result<(), Error> {
        Err(Error::Message(
            "codex interrupt is not sent into the pane".to_owned(),
        ))
    }

    async fn confirm_delivery(&self, seat: SeatId, text: &str) -> Result<(), Error> {
        let deadline = Instant::now() + self.budget().thread_id_after_submit;
        loop {
            match self.delivery_state(seat, text).await? {
                Delivery::Landed => return Ok(()),
                Delivery::Waiting => {
                    if Instant::now() >= deadline {
                        self.fail_id(seat, ReadinessCode::NotVerified).await;
                        return Err(Error::Message(
                            "rollout did not record the submit".to_owned(),
                        ));
                    }
                    tokio::time::sleep(self.budget().poll).await;
                }
                Delivery::Ambiguous => {
                    self.fail_id(seat, ReadinessCode::ThreadIdAmbiguous).await;
                    return Err(Error::Message("thread id is ambiguous".to_owned()));
                }
            }
        }
    }

    async fn delivery_state(&self, seat: SeatId, text: &str) -> Result<Delivery, Error> {
        let Some(query) = self.id_query(seat).await else {
            return Ok(Delivery::Waiting);
        };
        let body = text.to_owned();
        let resolved = resolve_for(&query, &body).await?;
        match resolved {
            Ok(id) => exact_user_item(self, seat, &query, id, &body).await,
            Err(ThreadIdError::NotYet) => Ok(Delivery::Waiting),
            Err(ThreadIdError::Ambiguous(_)) => Ok(Delivery::Ambiguous),
            Err(ThreadIdError::Malformed(path)) => Err(malformed(&path)),
        }
    }
}

enum Delivery {
    Landed,
    Waiting,
    Ambiguous,
}

async fn resolve_for(
    query: &IdQuery,
    body: &str,
) -> Result<Result<ThreadId, ThreadIdError>, Error> {
    let locator = query.locator.clone();
    let cwd = query.cwd.clone();
    let before = query.before.clone();
    let launched_at = query.launched_at;
    let probe = body.to_owned();
    blocking(move || locator.resolve(&before, &cwd, launched_at, Some(&probe))).await
}

async fn exact_user_item(
    adapter: &CodexAdapter,
    seat: SeatId,
    query: &IdQuery,
    id: ThreadId,
    body: &str,
) -> Result<Delivery, Error> {
    let locator = query.locator.clone();
    let cwd = query.cwd.clone();
    let wanted = id.clone();
    let exact = body.to_owned();
    let matched = blocking(move || match locator.verify_resume(&wanted, &cwd) {
        Ok(path) => user_turn_texts(&path).map(|turns| turns.iter().any(|turn| turn == &exact)),
        Err(ThreadIdError::NotYet) => Ok(false),
        Err(err) => Err(err),
    })
    .await?;
    match matched {
        Ok(true) => {
            adapter.store_resolved(seat, id).await;
            Ok(Delivery::Landed)
        }
        Ok(false) | Err(ThreadIdError::NotYet) => Ok(Delivery::Waiting),
        Err(ThreadIdError::Ambiguous(_)) => Ok(Delivery::Ambiguous),
        Err(ThreadIdError::Malformed(path)) => Err(malformed(&path)),
    }
}

fn draft_is_prefix(screen: &CodexScreen, body: &str) -> bool {
    let CodexScreen::Draft { visible } = screen else {
        return false;
    };
    !visible.is_empty() && body.starts_with(visible.as_str())
}

fn malformed(path: &std::path::Path) -> Error {
    Error::Message(format!("malformed rollout {}", path.display()))
}
