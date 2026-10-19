//! Paste into a ready Claude composer and confirm the transcript.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use crewmux_mux::{Capture, CaptureRange, Key, Multiplexer, PaneRef, PasteOpts};
use crewmux_transcript::{ClaudeTranscript, Delivery, TranscriptCursor, TranscriptSource};

use crate::adapter::SeatIo;
use crate::claude::screen::{ClaudeScreen, classify};
use crate::error::Error;
use crate::types::{DeliveryReceipt, SubmitMode};

/// One Claude pane. Enter is sent only after the classifier says ready.
pub struct ClaudeIo {
    mux: Arc<dyn Multiplexer>,
    source: Option<Arc<dyn TranscriptSource>>,
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
        source: Option<Arc<dyn TranscriptSource>>,
        target: IoTarget,
        submit_wait: Duration,
    ) -> Self {
        Self {
            mux,
            source,
            pane: target.pane,
            home: target.home,
            cwd: target.cwd,
            session_id: target.session_id,
            submit_wait,
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
        let landed = self.settled().await?;
        if !crate::claude::composer::composer_matches(&landed.text, text) {
            self.mux.send_keys(&self.pane, &[Key::CtrlU]).await?;
            return Err(Error::PasteMismatch);
        }
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
        Ok(self
            .mux
            .capture(&self.pane, CaptureRange::VisibleEscapes)
            .await?)
    }
}

impl ClaudeIo {
    async fn settled(&self) -> Result<Capture, Error> {
        let started = Instant::now();
        let mut previous = self.snapshot().await?;
        loop {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let next = self.snapshot().await?;
            if next.text == previous.text || started.elapsed() >= Duration::from_millis(300) {
                return Ok(next);
            }
            previous = next;
        }
    }

    async fn cursor(&self) -> Result<TranscriptCursor, Error> {
        self.with_source(|source| source.cursor()).await
    }

    async fn confirm(&self, sent: &str, after: TranscriptCursor) -> Result<(), Error> {
        // `/compact` is skipped in the transcript, so a user turn never appears.
        // Waiting out `submit_wait` only delays NotVerified. One probe still
        // reports a contamination that is already on disk.
        if compact_command(sent) {
            return self.probe(sent, after).await.and_then(landed_or_missing);
        }
        let deadline = Instant::now() + self.submit_wait;
        let sent = sent.to_owned();
        loop {
            let probe = sent.clone();
            let delivery = self
                .with_source(move |source| {
                    crewmux_transcript::verify_delivery(source, after, &probe)
                })
                .await?;
            match delivery {
                Delivery::Landed => return Ok(()),
                Delivery::Composite => return Err(Error::Contaminated),
                Delivery::Missing | Delivery::Duplicate(_) => {
                    if Instant::now() >= deadline {
                        return Err(Error::NotVerified);
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
        }
    }

    async fn probe(&self, sent: &str, after: TranscriptCursor) -> Result<Delivery, Error> {
        let probe = sent.to_owned();
        self.with_source(move |source| crewmux_transcript::verify_delivery(source, after, &probe))
            .await
    }

    async fn with_source<T, F>(&self, job: F) -> Result<T, Error>
    where
        T: Send + 'static,
        F: FnOnce(&dyn TranscriptSource) -> Result<T, crewmux_transcript::Error> + Send + 'static,
    {
        if let Some(source) = &self.source {
            let source = Arc::clone(source);
            return tokio::task::spawn_blocking(move || job(source.as_ref()))
                .await
                .map_err(|err| Error::Message(err.to_string()))?
                .map_err(map_transcript);
        }
        let home = self.home.clone();
        let cwd = self.cwd.clone();
        let session = self.session_id.clone();
        tokio::task::spawn_blocking(move || {
            let opened = ClaudeTranscript::open(&home, &cwd, &session)?;
            job(&opened)
        })
        .await
        .map_err(|err| Error::Message(err.to_string()))?
        .map_err(map_transcript)
    }
}

fn compact_command(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed == "/compact" || trimmed.starts_with("/compact ")
}

fn landed_or_missing(delivery: Delivery) -> Result<(), Error> {
    match delivery {
        Delivery::Landed => Ok(()),
        Delivery::Composite => Err(Error::Contaminated),
        Delivery::Missing | Delivery::Duplicate(_) => Err(Error::NotVerified),
    }
}

fn map_transcript(err: crewmux_transcript::Error) -> Error {
    match err {
        crewmux_transcript::Error::Io(err) => Error::Io(err),
        crewmux_transcript::Error::SessionId
        | crewmux_transcript::Error::Parse { .. }
        | crewmux_transcript::Error::Incomplete { .. } => Error::Message(err.to_string()),
    }
}
