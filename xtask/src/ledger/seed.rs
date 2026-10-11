//! Build the ledger from an `OpenRig` checkout.

use std::fs;
use std::path::Path;

use super::parse::to_csv;
use super::scope::waiver_for;
use super::types::{LedgerRow, LedgerStatus};
use crate::error::{Error, io_err};

const ROOTS: [&str; 4] = [
    "packages/daemon/test",
    "packages/cli/test",
    "packages/tui/test",
    "packages/test-system",
];

/// One row per `*.test.ts` / `*.spec.ts` (and `.tsx`) under the four roots.
/// Out-of-scope paths are `waived` with a note. Everything else is `todo`.
///
/// # Errors
///
/// Returns [`Error::Io`] when a directory cannot be read.
pub fn seed_csv(openrig: &Path) -> Result<String, Error> {
    let mut paths = Vec::new();
    for root in ROOTS {
        collect(&openrig.join(root), openrig, &mut paths)?;
    }
    paths.sort();
    let mut rows = Vec::new();
    for ts_path in paths {
        let waiver = waiver_for(&ts_path)?;
        let (status, note) = match waiver {
            Some(note) => (LedgerStatus::Waived, note.to_owned()),
            None => (LedgerStatus::Todo, String::new()),
        };
        rows.push(LedgerRow {
            ts_path,
            slice: None,
            status,
            rust_test: None,
            pe_id: None,
            note,
        });
    }
    Ok(to_csv(&rows))
}

fn collect(dir: &Path, openrig: &Path, out: &mut Vec<String>) -> Result<(), Error> {
    if !dir.exists() {
        return Ok(());
    }
    let entries =
        fs::read_dir(dir).map_err(|source| io_err(format!("read {}", dir.display()), source))?;
    for entry in entries {
        let entry = entry.map_err(|source| io_err("read test directory entry", source))?;
        let path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|source| io_err(format!("stat {}", path.display()), source))?;
        if kind.is_dir() {
            collect(&path, openrig, out)?;
        } else if kind.is_file() && is_test_file(&path) {
            let rel = path.strip_prefix(openrig).map_err(|_| {
                Error::Message(format!(
                    "{} is outside {}",
                    path.display(),
                    openrig.display()
                ))
            })?;
            let text = rel
                .to_str()
                .ok_or_else(|| Error::Message(format!("{} is not valid UTF-8", path.display())))?;
            out.push(text.replace('\\', "/"));
        }
    }
    Ok(())
}

fn is_test_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.ends_with(".test.ts")
        || name.ends_with(".test.tsx")
        || name.ends_with(".spec.ts")
        || name.ends_with(".spec.tsx")
}
