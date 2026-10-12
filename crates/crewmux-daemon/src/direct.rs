//! Direct-delivery rows and the stub agent jsonl.
//!
//! The pane transcript writer stays in [`crate::transcripts`]. This file is the
//! ADR-0009 user-turn log restore fingerprints against.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;

use crewmux_store::DirectRow;
use crewmux_transcript::{
    TranscriptCursor, TranscriptProbe, UserTurn, fingerprint, user_turns_after,
};
use crewmux_types::{DeliveryId, RuntimeKind, SeatId};
use serde_json::json;

use crate::error::Error;
use crate::query::find_address;
use crate::resume::agent_transcript;
use crate::state::{App, join_err, now_ms};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Before,
    Ack,
}

/// Crash-before-paste hook. Records the row and skips the paste.
///
/// # Errors
///
/// Returns [`Error`] when the seat or the row cannot be written.
pub(crate) async fn crash_before_paste(
    app: &App,
    address: &str,
    body: &str,
) -> Result<bool, Error> {
    if phase(app).await != Some(Phase::Before) {
        return Ok(false);
    }
    let seat = find_address(app, address).await?;
    let cursor = transcript_len(&app.home, &seat.id).await?;
    insert_row(app, &seat, body, None, false, cursor).await?;
    Ok(true)
}

/// Record a paste that landed. `ack` marks it confirmed.
///
/// # Errors
///
/// Returns [`Error`] when the transcript or the row cannot be written.
pub(crate) async fn note_pasted(app: &App, address: &str, body: &str) -> Result<(), Error> {
    let seat = find_address(app, address).await?;
    let acked = phase(app).await == Some(Phase::Ack);
    let cursor = append_turn(&app.home, &seat.id, body).await?;
    insert_row(
        app,
        &seat,
        body,
        Some(i64::try_from(now_ms()).unwrap_or(0)),
        acked,
        cursor,
    )
    .await
}

/// Append one user turn. Returns the byte cursor from before the append.
///
/// # Errors
///
/// Returns [`Error`] when the file cannot be written.
pub(crate) async fn append_turn(
    home: &std::path::Path,
    seat: &SeatId,
    body: &str,
) -> Result<u64, Error> {
    let path = agent_transcript(home, seat);
    let body = body.to_owned();
    tokio::task::spawn_blocking(move || append_line(&path, &body))
        .await
        .map_err(join_err)?
}

fn append_line(path: &Path, body: &str) -> Result<u64, Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let cursor = match std::fs::metadata(path) {
        Ok(meta) => meta.len(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => 0,
        Err(err) => return Err(err.into()),
    };
    let line = json!({"type":"user","message":{"role":"user","content": body}});
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{line}")?;
    Ok(cursor)
}

async fn insert_row(
    app: &App,
    seat: &crewmux_core::Seat,
    body: &str,
    delivered_at_ms: Option<i64>,
    acked: bool,
    cursor: u64,
) -> Result<(), Error> {
    let row = DirectRow {
        id: DeliveryId::generate().to_string(),
        rig_id: seat.rig_id.to_string(),
        seat_id: seat.id.to_string(),
        channel: "direct".to_owned(),
        fingerprint: fingerprint(seat.runtime, body).to_string(),
        body: body.to_owned(),
        delivered_at_ms,
        cursor: i64::try_from(cursor).ok(),
        acked,
    };
    let store = Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || {
        store.write(move |conn| crewmux_store::insert_direct(conn, &row))
    })
    .await
    .map_err(join_err)?
    .map_err(Error::from)
}

async fn transcript_len(home: &Path, seat: &SeatId) -> Result<u64, Error> {
    let path = agent_transcript(home, seat);
    tokio::task::spawn_blocking(move || match std::fs::metadata(path) {
        Ok(meta) => Ok(meta.len()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(err) => Err(Error::from(err)),
    })
    .await
    .map_err(join_err)?
}

async fn phase(app: &App) -> Option<Phase> {
    if let Some(token) = app.direct_phase().await {
        return phase_token(token);
    }
    if std::env::var("RIG_TEST_HOOKS").ok().as_deref() != Some("1") {
        return None;
    }
    let path = app.home.join("direct-phase");
    let text = tokio::task::spawn_blocking(move || std::fs::read_to_string(path).ok())
        .await
        .ok()??;
    phase_token(text.trim())
}

fn phase_token(token: &str) -> Option<Phase> {
    match token {
        "before" => Some(Phase::Before),
        "ack" => Some(Phase::Ack),
        _ => None,
    }
}

/// Read-only probe over stub agent jsonl files.
pub(crate) struct HomeProbe {
    home: std::path::PathBuf,
    runtimes: HashMap<SeatId, RuntimeKind>,
}

impl HomeProbe {
    pub(crate) fn new(home: std::path::PathBuf, seats: &[crewmux_core::Seat]) -> Self {
        let runtimes = seats.iter().map(|seat| (seat.id, seat.runtime)).collect();
        Self { home, runtimes }
    }
}

impl TranscriptProbe for HomeProbe {
    fn runtime(&self, seat: &SeatId) -> RuntimeKind {
        self.runtimes
            .get(seat)
            .copied()
            .unwrap_or(RuntimeKind::Stub)
    }

    fn user_turns_after(
        &self,
        seat: &SeatId,
        after: Option<TranscriptCursor>,
    ) -> Result<Vec<UserTurn>, crewmux_transcript::Error> {
        let path = agent_transcript(&self.home, seat);
        user_turns_after(&path, after.map_or(0, |cursor| cursor.0))
    }
}
