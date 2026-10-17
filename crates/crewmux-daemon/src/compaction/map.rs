//! Publish `RESTORE-MAP.md` when `RESTORE-MAP.md.tmp` ends with the marker.

use std::path::{Path, PathBuf};

use crewmux_health::compaction::{PrepStatus, completion_marker, map_is_complete, map_path};

use crate::error::Error;
use crate::query::find_seat;
use crate::state::{App, join_err};

pub(super) async fn publish(app: &App) -> Result<(), Error> {
    let pending = waiting(app).await;
    for (session, attempt) in pending {
        let Ok(seat) = find_seat(app, &session).await else {
            continue;
        };
        let path = map_path(&app.home, seat.cwd.as_deref(), &session, &attempt);
        let marker = completion_marker(&attempt, &session);
        if !marked(&path, &marker).await? {
            continue;
        }
        let mut guard = app.compaction.lock().await;
        if let Some(prep) = guard.preps.get_mut(&session)
            && prep.attempt_id == attempt
        {
            prep.map_complete = true;
        }
    }
    Ok(())
}

async fn waiting(app: &App) -> Vec<(String, String)> {
    let guard = app.compaction.lock().await;
    guard
        .preps
        .iter()
        .filter(|(_, prep)| {
            prep.active
                && !prep.map_complete
                && !prep.attempt_id.is_empty()
                && matches!(prep.status, PrepStatus::Waiting | PrepStatus::Sending)
        })
        .map(|(session, prep)| (session.clone(), prep.attempt_id.clone()))
        .collect()
}

async fn marked(path: &Path, marker: &str) -> Result<bool, Error> {
    let path = path.to_path_buf();
    let marker = marker.to_owned();
    tokio::task::spawn_blocking(move || publish_file(&path, &marker))
        .await
        .map_err(join_err)?
}

fn publish_file(path: &Path, marker: &str) -> Result<bool, Error> {
    if read_complete(path, marker) {
        return Ok(true);
    }
    let tmp = tmp_path(path);
    if is_symlink(&tmp) || !read_complete(&tmp, marker) {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(read_complete(path, marker))
}

fn read_complete(path: &Path, marker: &str) -> bool {
    std::fs::read_to_string(path).is_ok_and(|body| map_is_complete(&body, marker))
}

fn tmp_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.tmp", path.display()))
}

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}

#[cfg(test)]
mod tests {
    use super::{is_symlink, publish_file, tmp_path};
    use crewmux_health::compaction::completion_marker;

    #[test]
    fn tmp_with_marker_is_published() {
        let root = std::env::temp_dir().join(format!("rig-map-{}", std::process::id()));
        drop(std::fs::remove_dir_all(&root));
        std::fs::create_dir_all(&root).expect("dir");
        let path = root.join("RESTORE-MAP.md");
        let marker = completion_marker("attempt-1", "seat");
        let body = format!("# map\n{marker}\n");
        std::fs::write(tmp_path(&path), &body).expect("tmp");
        assert!(publish_file(&path, &marker).expect("publish"));
        assert_eq!(std::fs::read_to_string(&path).expect("final"), body);
        assert!(!tmp_path(&path).exists());
        drop(std::fs::remove_dir_all(&root));
    }

    #[test]
    fn symlinked_tmp_is_not_published() {
        let root = std::env::temp_dir().join(format!("rig-map-link-{}", std::process::id()));
        drop(std::fs::remove_dir_all(&root));
        std::fs::create_dir_all(&root).expect("dir");
        let path = root.join("RESTORE-MAP.md");
        let marker = completion_marker("attempt-1", "seat");
        let real = root.join("elsewhere.md");
        std::fs::write(&real, format!("# map\n{marker}\n")).expect("real");
        std::os::unix::fs::symlink(&real, tmp_path(&path)).expect("link");
        assert!(!publish_file(&path, &marker).expect("publish"));
        assert!(!path.exists());
        assert!(is_symlink(&tmp_path(&path)));
        drop(std::fs::remove_dir_all(&root));
    }
}
