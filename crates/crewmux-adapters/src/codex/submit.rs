//! Submit only while the composer is ready, then require a byte-exact rollout item.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crewmux_mux::PasteOpts;
use crewmux_types::{Key, ReadinessCode, SeatId, ThreadId};

use crewmux_transcript::{
    CodexRollout, Delivery as TurnDelivery, TranscriptCursor, verify_delivery,
};

use crate::codex::adapter::{CodexAdapter, IdQuery, blocking};
use crate::codex::rollout::ThreadIdError;
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
        let baseline = self.rollout_baseline(binding.seat).await?;
        self.mux()
            .paste(&binding.pane, text, PasteOpts::default())
            .await?;
        if mode == SubmitMode::Enter {
            let pasted = self.settled_draft(binding, text).await?;
            if !draft_accepts(&pasted, text) {
                self.clear_rejected_draft(binding, &pasted).await?;
                return Err(Error::Message(draft_error(&pasted, text)));
            }
            self.mux().send_keys(&binding.pane, &[Key::Enter]).await?;
        }
        self.confirm_delivery(binding.seat, text, &baseline).await?;
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

    async fn confirm_delivery(
        &self,
        seat: SeatId,
        text: &str,
        baseline: &BTreeMap<PathBuf, u64>,
    ) -> Result<(), Error> {
        let deadline = Instant::now() + self.budget().thread_id_after_submit;
        loop {
            match self.delivery_state(seat, text, baseline).await? {
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

    async fn rollout_baseline(&self, seat: SeatId) -> Result<BTreeMap<PathBuf, u64>, Error> {
        let Some(query) = self.id_query(seat).await else {
            return Ok(BTreeMap::new());
        };
        blocking(move || query.locator.byte_baseline()).await
    }

    async fn delivery_state(
        &self,
        seat: SeatId,
        text: &str,
        baseline: &BTreeMap<PathBuf, u64>,
    ) -> Result<Delivery, Error> {
        let Some(query) = self.id_query(seat).await else {
            return Ok(Delivery::Waiting);
        };
        let body = text.to_owned();
        let resolved = resolve_for(&query, &body).await?;
        match resolved {
            Ok(id) => exact_user_item(self, seat, &query, id, &body, baseline).await,
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
    baseline: &BTreeMap<PathBuf, u64>,
) -> Result<Delivery, Error> {
    let locator = query.locator.clone();
    let cwd = query.cwd.clone();
    let wanted = id.clone();
    let exact = body.to_owned();
    let lengths = baseline.clone();
    let matched = blocking(move || match locator.verify_resume(&wanted, &cwd) {
        Ok(path) => {
            let offset = lengths.get(&path).copied().unwrap_or(0);
            let rollout = CodexRollout::open(path);
            verify_delivery(&rollout, TranscriptCursor(offset), &exact)
                .map_err(|err| ThreadIdError::Malformed(rollout_path(&err)))
        }
        Err(ThreadIdError::NotYet) => Ok(TurnDelivery::Missing),
        Err(err) => Err(err),
    })
    .await?;
    match matched {
        Ok(TurnDelivery::Landed) => {
            adapter.store_resolved(seat, id).await;
            Ok(Delivery::Landed)
        }
        Ok(TurnDelivery::Missing) | Err(ThreadIdError::NotYet) => Ok(Delivery::Waiting),
        Ok(TurnDelivery::Composite | TurnDelivery::Duplicate(_)) => Err(Error::Message(
            "rollout did not record a single submit".to_owned(),
        )),
        Err(ThreadIdError::Ambiguous(_)) => Ok(Delivery::Ambiguous),
        Err(ThreadIdError::Malformed(path)) => Err(malformed(&path)),
    }
}

fn rollout_path(err: &crewmux_transcript::Error) -> std::path::PathBuf {
    match err {
        crewmux_transcript::Error::Parse { path, .. }
        | crewmux_transcript::Error::Incomplete { path } => path.clone(),
        crewmux_transcript::Error::Io(_) | crewmux_transcript::Error::SessionId => {
            std::path::PathBuf::from("rollout")
        }
    }
}

impl CodexAdapter {
    /// Wait until the composer stops changing, then return that screen.
    ///
    /// Codex collapses a paste over 1000 characters after the bracket closes.
    /// Checking the first frame accepts a prefix that is about to be replaced,
    /// or rejects the placeholder before it is drawn.
    async fn settled_draft(&self, binding: &NodeBinding, text: &str) -> Result<CodexScreen, Error> {
        let deadline = Instant::now() + paste_window(self.budget().poll);
        let mut seen = self.visible_screen(binding).await?;
        loop {
            if Instant::now() >= deadline {
                return Ok(seen);
            }
            tokio::time::sleep(self.budget().poll).await;
            let next = self.visible_screen(binding).await?;
            if next == seen && (draft_accepts(&next, text) || visible_draft(&next)) {
                return Ok(next);
            }
            seen = next;
        }
    }

    async fn clear_rejected_draft(
        &self,
        binding: &NodeBinding,
        screen: &CodexScreen,
    ) -> Result<(), Error> {
        if visible_draft(screen) {
            self.mux().send_keys(&binding.pane, &[Key::CtrlU]).await?;
        }
        Ok(())
    }
}

fn paste_window(poll: Duration) -> Duration {
    let scaled = poll.saturating_mul(30);
    if scaled < Duration::from_millis(250) {
        Duration::from_millis(250)
    } else {
        scaled
    }
}

fn visible_draft(screen: &CodexScreen) -> bool {
    matches!(screen, CodexScreen::Draft { visible } if !visible.is_empty())
}

fn draft_accepts(screen: &CodexScreen, body: &str) -> bool {
    let CodexScreen::Draft { visible } = screen else {
        return false;
    };
    if visible.is_empty() {
        return false;
    }
    body.starts_with(visible.as_str()) || pasted_placeholder_matches(visible, body)
}

/// `[Pasted Content N chars]` counts Unicode scalar values, not bytes.
fn pasted_placeholder_matches(visible: &str, body: &str) -> bool {
    pasted_content_count(visible).is_some_and(|count| count == body.chars().count())
}

fn pasted_content_count(visible: &str) -> Option<usize> {
    let rest = visible.trim().strip_prefix("[Pasted Content ")?;
    let (digits, tail) = rest.split_once(" chars]")?;
    if !tail.is_empty() || digits.is_empty() || !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn draft_error(screen: &CodexScreen, body: &str) -> String {
    if let CodexScreen::Draft { visible } = screen
        && pasted_content_count(visible).is_some()
        && !pasted_placeholder_matches(visible, body)
    {
        return "paste placeholder count did not match".to_owned();
    }
    "composer did not show the paste".to_owned()
}

fn malformed(path: &std::path::Path) -> Error {
    Error::Message(format!("malformed rollout {}", path.display()))
}
