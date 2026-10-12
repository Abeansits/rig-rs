//! Resume files and the posture recorded for a later restore.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crewmux_adapters::ResumeToken;
use crewmux_durability::{parse_posture, posture_token};
use crewmux_store::SeatRestoreRow;
use crewmux_types::{LaunchPosture, SeatId};

use crate::error::Error;
use crate::state::{App, join_err};

/// Extra launch inputs. `up` uses [`LaunchExtra::initial`].
pub(crate) struct LaunchExtra {
    /// Posture the adapter must not widen past.
    pub posture: LaunchPosture,
    /// Resume token, when restore found one.
    pub resume: Option<ResumeToken>,
    /// Transcript lines the stub prints on resume.
    pub replay: Option<PathBuf>,
}

impl LaunchExtra {
    /// First launch. Locked and floor come from the seat name.
    pub(crate) fn initial(name: &str) -> Self {
        Self {
            posture: posture_for_name(name),
            resume: None,
            replay: None,
        }
    }
}

/// `locked-*` and `floor-*` seats keep that posture. Others start at auto.
pub(crate) fn posture_for_name(name: &str) -> LaunchPosture {
    if name.starts_with("locked-") {
        LaunchPosture::Locked
    } else if name.starts_with("floor-") {
        LaunchPosture::Floor
    } else {
        LaunchPosture::Auto
    }
}

/// Stub script, plus a trust gate for seats named `gate-*`.
pub(crate) fn script_for(app: &App, name: &str) -> Option<String> {
    let base = std::env::var("RIG_STUB_SCRIPT")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| app.stub_script.clone())
        .unwrap_or_default();
    if name.starts_with("gate-") {
        return Some(with_gate(&base));
    }
    if base.is_empty() { None } else { Some(base) }
}

fn with_gate(base: &str) -> String {
    const GATE: &str = "print_gate=trust_gate";
    if base.is_empty() {
        GATE.to_owned()
    } else if base.contains("print_gate=") {
        base.to_owned()
    } else {
        format!("{base},{GATE}")
    }
}

/// `$RIG_HOME/resume/<seat>.token`.
pub(crate) fn resume_path(home: &Path, seat: &SeatId) -> PathBuf {
    home.join("resume").join(format!("{seat}.token"))
}

/// `$RIG_HOME/resume/<seat>.replay`.
pub(crate) fn replay_path(home: &Path, seat: &SeatId) -> PathBuf {
    home.join("resume").join(format!("{seat}.replay"))
}

/// Agent jsonl used for delivery fingerprints. Not the pane transcript writer.
pub(crate) fn agent_transcript(home: &Path, seat: &SeatId) -> PathBuf {
    home.join("transcripts")
        .join("stub")
        .join(format!("{seat}.jsonl"))
}

/// Mint the stub resume file and store the posture.
///
/// # Errors
///
/// Returns [`Error`] when the file or the row cannot be written.
pub(crate) async fn remember(
    app: &App,
    seat: &crewmux_core::Seat,
    posture: LaunchPosture,
) -> Result<(), Error> {
    let path = resume_path(&app.home, &seat.id);
    let token = format!("stub-{}", seat.id);
    let row = SeatRestoreRow {
        seat_id: seat.id.to_string(),
        posture: posture_token(posture).to_owned(),
        resume_kind: Some("stub".to_owned()),
        resume_value: Some(token.clone()),
        transcript_cursor: None,
        startup_status: None,
    };
    let store = Arc::clone(&app.store);
    tokio::task::spawn_blocking(move || write_resume(&path, &token, &store, row))
        .await
        .map_err(join_err)?
}

fn write_resume(
    path: &Path,
    token: &str,
    store: &crewmux_store::Store,
    row: SeatRestoreRow,
) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, token)?;
    store
        .write(move |conn| crewmux_store::put_seat_restore(conn, &row))
        .map_err(Error::from)
}

/// Posture stored for a seat, or the name-based default.
///
/// # Errors
///
/// Returns [`Error`] when the row cannot be read.
pub(crate) async fn recorded_posture(
    app: &App,
    seat: &crewmux_core::Seat,
) -> Result<LaunchPosture, Error> {
    let id = seat.id.to_string();
    let fallback = posture_for_name(seat.name.as_str());
    let store = Arc::clone(&app.store);
    let row = tokio::task::spawn_blocking(move || {
        store.read(move |conn| crewmux_store::load_seat_restore(conn, &id))
    })
    .await
    .map_err(join_err)??;
    Ok(row.map_or(fallback, |row| parse_posture(&row.posture)))
}
