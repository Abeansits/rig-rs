//! Snapshot manifest and the sha256 check restore refuses on.

use crewmux_types::{LaunchPosture, RigId, RuntimeKind, SeatId, SnapshotId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::Error;
use crate::reconcile::InFlightDelivery;

/// Resume token captured with a seat. Adapters keep their own token type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredResume {
    /// Token family (`stub`, `claude`, `codex`).
    pub kind: String,
    /// Token value.
    pub value: String,
}

/// One seat inside a snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatSnapshot {
    /// Seat id.
    pub seat: SeatId,
    /// Seat name, for operators. The id is the identity.
    pub name: String,
    /// Runtime that owned the seat.
    pub runtime: RuntimeKind,
    /// Resume token, when one had been minted.
    pub resume: Option<StoredResume>,
    /// Transcript byte cursor.
    pub transcript_cursor: Option<u64>,
    /// Posture at capture. Restore will not widen it.
    pub posture: LaunchPosture,
    /// Startup delivery status, when one was recorded.
    pub startup_status: Option<String>,
}

/// Manifest stored next to the SQLite backup.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotManifest {
    /// Snapshot id.
    pub id: SnapshotId,
    /// Rig.
    pub rig: RigId,
    /// Rig name.
    pub rig_name: String,
    /// Unix milliseconds.
    pub created_at: i64,
    /// `PRAGMA user_version` of the backup.
    pub schema_version: u32,
    /// sha256 of the backup file.
    pub sha256: String,
    /// Seats captured with the rig.
    pub seats: Vec<SeatSnapshot>,
    /// In-flight queue rows and direct deliveries.
    pub in_flight: Vec<InFlightDelivery>,
}

/// Lower-case hex sha256.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        push_hex(&mut out, byte);
    }
    out
}

fn push_hex(out: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let hi = usize::from(byte >> 4);
    let lo = usize::from(byte & 0x0f);
    if let Some(ch) = HEX.get(hi).copied() {
        out.push(char::from(ch));
    }
    if let Some(ch) = HEX.get(lo).copied() {
        out.push(char::from(ch));
    }
}

/// Refuse a snapshot whose hash does not match, or whose schema is newer.
///
/// # Errors
///
/// Returns [`Error::Corrupt`] or [`Error::NewerSchema`].
pub fn verify_snapshot(
    manifest: &SnapshotManifest,
    backup_sha: &str,
    file_schema: u32,
    supported: u32,
) -> Result<(), Error> {
    let found = manifest.schema_version.max(file_schema);
    if found > supported {
        return Err(Error::NewerSchema { found, supported });
    }
    if manifest.sha256 != backup_sha {
        return Err(Error::Corrupt {
            id: manifest.id.to_string(),
            expected: manifest.sha256.clone(),
            found: backup_sha.to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{SnapshotManifest, sha256_hex, verify_snapshot};
    use crewmux_types::{RigId, SnapshotId};

    fn manifest(sha: &str, schema: u32) -> SnapshotManifest {
        SnapshotManifest {
            id: SnapshotId::new(),
            rig: RigId::new(),
            rig_name: "pair".to_owned(),
            created_at: 0,
            schema_version: schema,
            sha256: sha.to_owned(),
            seats: Vec::new(),
            in_flight: Vec::new(),
        }
    }

    #[test]
    fn corrupted_bytes_are_refused() {
        let sha = sha256_hex(b"backup");
        let manifest = manifest(&sha, 4);
        let err = verify_snapshot(&manifest, &sha256_hex(b"tampered"), 4, 4).expect_err("corrupt");
        assert!(err.to_string().contains("corrupted"), "{err}");
    }

    #[test]
    fn newer_schema_is_refused() {
        let sha = sha256_hex(b"backup");
        let manifest = manifest(&sha, 9);
        let err = verify_snapshot(&manifest, &sha, 9, 4).expect_err("newer");
        assert!(err.to_string().contains("newer"), "{err}");
    }
}
