//! Drain `<seat>/hooks.spool.jsonl` in order. A partial last line stays on disk.

use std::path::{Path, PathBuf};

use crewmux_health::compaction::{parse_spool_line, split_spool_text};

use crate::error::Error;
use crate::state::{App, join_err};

use super::apply;

pub(crate) async fn drain_spools(app: &App) -> Result<(), Error> {
    let home = app.home.clone();
    let files = tokio::task::spawn_blocking(move || spool_files(&home))
        .await
        .map_err(join_err)??;
    for (seat_key, path) in files {
        drain_one(app, &seat_key, &path).await?;
    }
    Ok(())
}

async fn drain_one(app: &App, seat_key: &str, path: &Path) -> Result<(), Error> {
    let reading = path.to_path_buf();
    let text = tokio::task::spawn_blocking(move || std::fs::read_to_string(&reading))
        .await
        .map_err(join_err)??;
    let split = split_spool_text(&text);
    for line in &split.complete {
        let Some(event) = parse_spool_line(line) else {
            tracing::warn!(seat_key, "skipped malformed spool line");
            continue;
        };
        let bytes = serde_json::to_vec(&serde_json::json!({
            "run_id": event.run_id,
            "payload": event.payload,
        }))
        .map_err(|err| Error::Message(err.to_string()))?;
        apply::ingest(app, seat_key, &bytes).await?;
    }
    let writing = path.to_path_buf();
    let remainder = split.remainder;
    tokio::task::spawn_blocking(move || rewrite(&writing, &remainder))
        .await
        .map_err(join_err)?
}

fn rewrite(path: &Path, remainder: &str) -> Result<(), Error> {
    if remainder.is_empty() {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        return Ok(());
    }
    std::fs::write(path, remainder)?;
    Ok(())
}

fn spool_files(home: &Path) -> Result<Vec<(String, PathBuf)>, Error> {
    let seats = home.join("seats");
    if !seats.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&seats)? {
        let entry = entry?;
        let path = entry.path().join("hooks.spool.jsonl");
        if path.is_file() {
            out.push((entry.file_name().to_string_lossy().into_owned(), path));
        }
    }
    out.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(out)
}
