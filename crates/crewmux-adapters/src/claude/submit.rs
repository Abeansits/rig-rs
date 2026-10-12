//! Paste into a ready Claude composer and confirm the transcript.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use crewmux_mux::{Capture, CaptureRange, Key, Multiplexer, PaneRef, PasteOpts};

use crate::adapter::SeatIo;
use crate::claude::screen::{ClaudeScreen, classify};
use crate::delivery::{DeliveryCheck, ProbeQuery, TurnVerdict};
use crate::error::Error;
use crate::types::{DeliveryReceipt, SubmitMode};

/// One Claude pane. Enter is sent only after the classifier says ready.
pub struct ClaudeIo {
    mux: Arc<dyn Multiplexer>,
    check: Arc<dyn DeliveryCheck>,
    pane: PaneRef,
    home: PathBuf,
    cwd: PathBuf,
    session_id: String,
    submit_wait: Duration,
}

pub(crate) struct IoTarget {
    pub pane: PaneRef,
    pub home: PathBuf,
    pub cwd: PathBuf,
    pub session_id: String,
}

impl ClaudeIo {
    pub(crate) fn new(
        mux: Arc<dyn Multiplexer>,
        check: Arc<dyn DeliveryCheck>,
        target: IoTarget,
        submit_wait: Duration,
    ) -> Self {
        Self {
            mux,
            check,
            pane: target.pane,
            home: target.home,
            cwd: target.cwd,
            session_id: target.session_id,
            submit_wait,
        }
    }

    fn query(&self, after_bytes: u64) -> ProbeQuery {
        ProbeQuery {
            home: self.home.clone(),
            cwd: self.cwd.clone(),
            session_id: self.session_id.clone(),
            after_bytes,
        }
    }
}

#[async_trait]
impl SeatIo for ClaudeIo {
    async fn submit(&self, text: &str, mode: SubmitMode) -> Result<DeliveryReceipt, Error> {
        let capture = self.snapshot().await?;
        if !matches!(classify(&capture), ClaudeScreen::Ready(_)) {
            return Err(Error::NotReady);
        }
        let cursor = self.cursor().await?;
        self.mux
            .paste(&self.pane, text, PasteOpts::default())
            .await?;
        if mode == SubmitMode::Enter {
            self.mux.send_keys(&self.pane, &[Key::Enter]).await?;
        }
        self.confirm(text, cursor).await?;
        Ok(DeliveryReceipt { bytes: text.len() })
    }

    async fn interrupt(&self) -> Result<(), Error> {
        Err(Error::Message(
            "claude panes are stopped through tmux, not a quit key".to_owned(),
        ))
    }

    async fn snapshot(&self) -> Result<Capture, Error> {
        Ok(self.mux.capture(&self.pane, CaptureRange::Visible).await?)
    }
}

impl ClaudeIo {
    async fn cursor(&self) -> Result<u64, Error> {
        let query = self.query(0);
        let check = Arc::clone(&self.check);
        tokio::task::spawn_blocking(move || check.cursor(&query))
            .await
            .map_err(|err| Error::Message(err.to_string()))?
    }

    async fn confirm(&self, sent: &str, after: u64) -> Result<(), Error> {
        let deadline = Instant::now() + self.submit_wait;
        let sent = sent.to_owned();
        loop {
            let turns = self.turns(after).await?;
            if verdict_is(&*self.check, &sent, &turns, TurnVerdict::Composite) {
                return Err(Error::Contaminated);
            }
            let matched = count_verdict(&*self.check, &sent, &turns, TurnVerdict::Equal);
            if matched == 1 && turns.len() == 1 {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(Error::NotVerified);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn turns(&self, after: u64) -> Result<Vec<String>, Error> {
        let query = self.query(after);
        let check = Arc::clone(&self.check);
        tokio::task::spawn_blocking(move || check.turns(&query))
            .await
            .map_err(|err| Error::Message(err.to_string()))?
    }
}

fn verdict_is(check: &dyn DeliveryCheck, sent: &str, turns: &[String], want: TurnVerdict) -> bool {
    turns.iter().any(|turn| check.verdict(sent, turn) == want)
}

fn count_verdict(
    check: &dyn DeliveryCheck,
    sent: &str,
    turns: &[String],
    want: TurnVerdict,
) -> usize {
    turns
        .iter()
        .filter(|turn| check.verdict(sent, turn) == want)
        .count()
}
