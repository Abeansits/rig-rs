//! Per-attempt restore markers.
//!
//! The file is `${RIG_HOME}/compaction/restore-pending/<seat_key>/<attempt_id>.json`.
//! A marker is acted on only when `confirmed` is true, which the daemon sets
//! after both completion signals. An unsolicited `PreCompact` writes a different
//! attempt file and leaves a managed marker byte-for-byte alone.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::hook::CompactTrigger;
use crate::error::Error;

/// One restore marker on disk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreMarker {
    /// Seat the attempt belongs to.
    pub seat_key: String,
    /// Attempt that owns this file.
    pub attempt_id: String,
    /// Manual or automatic.
    pub trigger: CompactTrigger,
    /// True when the enforcer requested `/compact`.
    pub managed: bool,
    /// Set only when `SessionStart{source=compact}` and `PostCompact` both arrived.
    pub confirmed: bool,
    /// Claude session id from the hook.
    pub session_id: String,
    /// Transcript path from `PreCompact`.
    pub transcript_path: PathBuf,
    /// Working directory from the hook, when it sent one.
    pub cwd: PathBuf,
}

/// The TypeScript session-key sanitiser: anything outside `[A-Za-z0-9_.@-]` becomes `_`.
#[must_use]
pub fn sanitize_seat_key(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '@' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

/// Marker path for one attempt. Both path segments are sanitised.
#[must_use]
pub fn marker_path(rig_home: &Path, seat_key: &str, attempt_id: &str) -> PathBuf {
    rig_home
        .join("compaction")
        .join("restore-pending")
        .join(sanitize_seat_key(seat_key))
        .join(format!("{}.json", sanitize_seat_key(attempt_id)))
}

/// Pretty JSON plus a trailing newline, matching the TypeScript writer.
///
/// # Errors
///
/// Returns [`Error::Message`] when the marker cannot be encoded.
pub fn marker_bytes(marker: &RestoreMarker) -> Result<Vec<u8>, Error> {
    let mut text =
        serde_json::to_string_pretty(marker).map_err(|err| Error::Message(err.to_string()))?;
    text.push('\n');
    Ok(text.into_bytes())
}

/// Read a marker document.
///
/// # Errors
///
/// Returns [`Error::Message`] when `bytes` are not a marker.
pub fn parse_marker(bytes: &[u8]) -> Result<RestoreMarker, Error> {
    serde_json::from_slice(bytes).map_err(|err| Error::Message(err.to_string()))
}

/// True only for a marker the daemon has confirmed.
#[must_use]
pub fn is_actionable(marker: &RestoreMarker) -> bool {
    marker.confirmed
}
