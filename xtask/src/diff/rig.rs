//! Register a rig name on the rig-rs daemon for a differential case.
//!
//! One authored spec cannot bring stub seats up on both daemons (PE-007).
//! TypeScript admits `member@rig` once `POST /api/rigs` has created the name.
//! rig-rs lists a rig only when it has a seat, so this inserts the rig plus a
//! `witness` seat that is not a workflow target. Missing preferred targets
//! then produce the same advisories on both sides.

use std::process::{Command, Stdio};

use super::types::HermeticHome;
use crate::error::{Error, io_err};

const SEED: &str = r#"
import sqlite3, sys
path, name = sys.argv[1], sys.argv[2]
rig_id = "diff-rig-" + name
seat_id = "diff-seat-" + name
conn = sqlite3.connect(path, timeout=5)
conn.execute("PRAGMA busy_timeout=5000")
conn.execute(
    "INSERT INTO rigs (id, name, created_ms) VALUES (?, ?, 0) ON CONFLICT(name) DO NOTHING",
    (rig_id, name),
)
row = conn.execute("SELECT id FROM rigs WHERE name = ?", (name,)).fetchone()
if row is None:
    raise SystemExit("rig row missing after insert")
conn.execute(
    "INSERT OR IGNORE INTO seats (id, rig_id, name, runtime, state) VALUES (?, ?, 'witness', 'claude', 'stopped')",
    (seat_id, row[0]),
)
conn.commit()
"#;

/// Insert `name` and a non-target `witness` seat into the running daemon's database.
///
/// # Errors
///
/// Returns [`Error`] when the database is missing or `python3` cannot commit the rows.
pub(crate) fn seed_witness(home: &HermeticHome, name: &str) -> Result<(), Error> {
    let db = home.rig_home().join("state/rig.db");
    if !db.is_file() {
        return Err(Error::Message(format!(
            "workflow diff expected {} after the daemon started",
            db.display()
        )));
    }
    let mut command = Command::new("python3");
    command
        .arg("-c")
        .arg(SEED)
        .arg(&db)
        .arg(name)
        .env_clear()
        .envs(home.child_env())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command
        .spawn()
        .map_err(|source| io_err("spawn python3", source))?;
    let output = child
        .wait_with_output()
        .map_err(|source| io_err("wait for python3", source))?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr);
    Err(Error::Message(format!(
        "seeding rig {name} failed: {detail}"
    )))
}
