//! Insta snapshots of the TypeScript side. The rig-rs side is never accepted.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, io_err};

/// `tests/diff/snapshots/<id>.ts.snap`.
#[must_use]
pub(crate) fn ts_snapshot_path(dir: &Path, case_id: &str) -> PathBuf {
    dir.join(format!("{case_id}.ts.snap"))
}

/// Write one TypeScript golden. Refuses any id that could escape the snapshot directory.
///
/// # Errors
///
/// Returns [`Error::Message`] for an unsafe id and [`Error::Io`] on write failure.
pub fn write_ts_golden(dir: &Path, case_id: &str, canonical: &str) -> Result<(), Error> {
    ensure_safe_id(case_id)?;
    fs::create_dir_all(dir).map_err(|source| io_err("create snapshot dir", source))?;
    let path = ts_snapshot_path(dir, case_id);
    let body = format!(
        "---\nsource: tests/diff/cases/{case_id}.json\nexpression: \"ts:{case_id}\"\n---\n{canonical}"
    );
    fs::write(&path, body).map_err(|source| io_err(format!("write {}", path.display()), source))?;
    Ok(())
}

/// Body of a TypeScript golden, after the insta front matter.
///
/// # Errors
///
/// Returns [`Error::Io`] when the file is missing and [`Error::Message`] when the
/// front matter is not an insta header.
pub fn read_ts_golden(dir: &Path, case_id: &str) -> Result<String, Error> {
    ensure_safe_id(case_id)?;
    let path = ts_snapshot_path(dir, case_id);
    let text = fs::read_to_string(&path)
        .map_err(|source| io_err(format!("read {}", path.display()), source))?;
    snapshot_body(&text)
}

pub(crate) fn snapshot_body(text: &str) -> Result<String, Error> {
    let rest = text
        .strip_prefix("---\n")
        .ok_or_else(|| Error::Message("insta snapshot is missing its opening header".to_owned()))?;
    let (_, body) = rest
        .split_once("\n---\n")
        .ok_or_else(|| Error::Message("insta snapshot is missing its closing header".to_owned()))?;
    Ok(body.to_owned())
}

fn ensure_safe_id(case_id: &str) -> Result<(), Error> {
    let ok = !case_id.is_empty()
        && case_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.');
    if ok {
        Ok(())
    } else {
        Err(Error::Message(format!(
            "case id {case_id} cannot be used as a snapshot file name"
        )))
    }
}
