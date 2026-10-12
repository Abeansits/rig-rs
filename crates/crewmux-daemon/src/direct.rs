//! Direct and startup delivery rows.
//!
//! Reconciliation reads the pane transcript written by [`crate::transcripts`],
//! not a jsonl this module appends.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crewmux_store::DirectRow;
use crewmux_transcript::{TranscriptCursor, TranscriptProbe, UserTurn, fingerprint};
use crewmux_types::{DeliveryId, RuntimeKind, SeatId};
use serde_json::Value;

use crate::error::Error;
use crate::query::find_address;
use crate::state::{App, join_err, now_ms};
use crate::turns::envelope_turns;

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
    let cursor = crate::transcripts::len_of(&app.home, seat.name.as_str()).await?;
    insert_row(
        app,
        &seat,
        Written {
            channel: "direct",
            body,
            delivered_at_ms: None,
            acked: false,
            cursor,
        },
    )
    .await?;
    Ok(true)
}

/// Record a paste that landed. `ack` marks it confirmed.
///
/// # Errors
///
/// Returns [`Error`] when the transcript or the row cannot be written.
pub(crate) async fn note_pasted(
    app: &App,
    address: &str,
    body: &str,
    cursor: u64,
) -> Result<(), Error> {
    let seat = find_address(app, address).await?;
    let acked = phase(app).await == Some(Phase::Ack);
    insert_row(
        app,
        &seat,
        Written {
            channel: "direct",
            body,
            delivered_at_ms: Some(i64::try_from(now_ms()).unwrap_or(0)),
            acked,
            cursor,
        },
    )
    .await
}

/// Record startup-file bodies that were pasted on a fresh launch.
///
/// # Errors
///
/// Returns [`Error`] when a startup paste or the row cannot be written.
pub(crate) async fn record_startup(app: &App, seats: &[crewmux_core::Seat]) -> Result<(), Error> {
    for seat in seats {
        let path = startup_path(&app.home, seat.name.as_str());
        let Some(body) = read_startup(&path).await? else {
            continue;
        };
        let cursor = crate::transcripts::len_of(&app.home, seat.name.as_str()).await?;
        let address = crate::query::address_of(app, seat).await?;
        let report = crate::deliver::deliver(app, startup_outbound(&address, &body)).await?;
        if !delivery_confirmed(&report) {
            continue;
        }
        insert_row(
            app,
            seat,
            Written {
                channel: "startup",
                body: &body,
                delivered_at_ms: Some(i64::try_from(now_ms()).unwrap_or(0)),
                acked: false,
                cursor,
            },
        )
        .await?;
    }
    Ok(())
}

fn startup_path(home: &Path, name: &str) -> std::path::PathBuf {
    home.join("startup").join(format!("{name}.txt"))
}

async fn read_startup(path: &Path) -> Result<Option<String>, Error> {
    let path = path.to_path_buf();
    let text = tokio::task::spawn_blocking(move || match std::fs::read_to_string(&path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(Error::from(err)),
    })
    .await
    .map_err(join_err)??;
    Ok(text.and_then(|text| {
        let trimmed = text.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    }))
}

fn startup_outbound(address: &str, body: &str) -> crate::deliver::Outbound {
    crate::deliver::Outbound {
        from: "startup@system".to_owned(),
        targets: vec![address.to_owned()],
        rig: None,
        all: false,
        body: body.to_owned(),
        interact: false,
        reason: None,
        verify: false,
        track_direct: false,
    }
}

/// True when every receipt in a [`crate::deliver::deliver`] result landed.
#[must_use]
pub(crate) fn delivery_confirmed(value: &Value) -> bool {
    if value.get("status").and_then(Value::as_str) != Some("ok") {
        return false;
    }
    let Some(rows) = value.get("receipts").and_then(Value::as_array) else {
        return false;
    };
    !rows.is_empty()
        && rows
            .iter()
            .all(|row| row.get("outcome").and_then(Value::as_str) == Some("delivered"))
}

struct Written<'a> {
    channel: &'a str,
    body: &'a str,
    delivered_at_ms: Option<i64>,
    acked: bool,
    cursor: u64,
}

async fn insert_row(
    app: &App,
    seat: &crewmux_core::Seat,
    written: Written<'_>,
) -> Result<(), Error> {
    let row = DirectRow {
        id: DeliveryId::generate().to_string(),
        rig_id: seat.rig_id.to_string(),
        seat_id: seat.id.to_string(),
        channel: written.channel.to_owned(),
        fingerprint: fingerprint(seat.runtime, written.body).to_string(),
        body: written.body.to_owned(),
        delivered_at_ms: written.delivered_at_ms,
        cursor: i64::try_from(written.cursor).ok(),
        acked: written.acked,
    };
    let store = Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || {
        store.write(move |conn| crewmux_store::insert_direct(conn, &row))
    })
    .await
    .map_err(join_err)?
    .map_err(Error::from)
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

/// Read-only probe over pane transcripts (`transcripts/<seat>/<gen>.txt`).
pub(crate) struct HomeProbe {
    home: std::path::PathBuf,
    names: HashMap<SeatId, String>,
    runtimes: HashMap<SeatId, RuntimeKind>,
}

impl HomeProbe {
    pub(crate) fn new(home: std::path::PathBuf, seats: &[crewmux_core::Seat]) -> Self {
        let names = seats
            .iter()
            .map(|seat| (seat.id, seat.name.as_str().to_owned()))
            .collect();
        let runtimes = seats.iter().map(|seat| (seat.id, seat.runtime)).collect();
        Self {
            home,
            names,
            runtimes,
        }
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
        let Some(name) = self.names.get(seat) else {
            return Ok(Vec::new());
        };
        let bytes = crate::transcripts::plain_bytes(&self.home, name)?;
        Ok(envelope_turns(&bytes, after.map_or(0, |cursor| cursor.0)))
    }
}
