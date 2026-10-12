//! Mint a session id, write per-seat files, then start Claude in the pane.

use std::path::{Path, PathBuf};

use crewmux_mux::{Geometry, SessionSpec};
use crewmux_provider::{ClaudeSeatFiles, ClaudeSeatSettings, RunHooks};

use crate::claude::argv::{claude_argv, mint_session_id};
use crate::error::Error;
use crate::types::{LaunchOpts, NodeBinding, ResumeToken};

pub(crate) struct Prepared {
    pub(crate) spec: SessionSpec,
    pub(crate) session_id: String,
}

pub(crate) async fn prepare(
    binding: &NodeBinding,
    opts: &LaunchOpts,
    seat_dir: &Path,
) -> Result<Prepared, Error> {
    let mode = binding.permission_mode.clone();
    let dir = seat_dir.to_path_buf();
    let seat_key = binding.seat.to_string();
    let rig_home = rig_home_of(seat_dir);
    let (mcp, settings) = tokio::task::spawn_blocking(move || {
        write_seat(&dir, mode.as_deref(), &seat_key, &rig_home)
    })
    .await
    .map_err(|err| Error::Message(err.to_string()))??;
    let session_id = match &opts.resume {
        Some(token) => crewmux_transcript::canonical_session_id(&token.value)
            .map_err(|_| Error::Message("resume token is not a UUID".to_owned()))?,
        None => mint_session_id(),
    };
    write_id(&seat_dir.join("claude-session-id"), &session_id).await?;
    let binary = opts
        .claude_bin
        .as_deref()
        .and_then(|path| path.to_str())
        .unwrap_or("claude");
    let mut spec = SessionSpec::new(opts.session_name.clone(), binding.cwd.clone());
    spec.env.clone_from(&opts.env);
    spec.argv = claude_argv(
        binary,
        &session_id,
        opts.resume.is_some(),
        &mcp,
        &settings,
        binding.model.as_deref(),
    );
    spec.geometry = Geometry { cols: 90, rows: 27 };
    spec.remain_on_exit = true;
    Ok(Prepared { spec, session_id })
}

pub(crate) fn token(session_id: String) -> ResumeToken {
    ResumeToken {
        kind: "claude".to_owned(),
        value: session_id,
    }
}

fn write_seat(
    dir: &Path,
    permission_mode: Option<&str>,
    seat_key: &str,
    rig_home: &Path,
) -> Result<(PathBuf, PathBuf), crewmux_provider::ProviderError> {
    let files = ClaudeSeatFiles::new(dir);
    let mcp = files.write_mcp(&[])?;
    let _seat_settings = files.write_settings(&ClaudeSeatSettings {
        permission_mode: permission_mode.map(str::to_owned),
        hooks: serde_json::json!({}),
    })?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let settings = files.write_run_settings(
        &RunHooks {
            seat_key: seat_key.to_owned(),
            rig_home: rig_home.to_path_buf(),
            run_id,
            rig_bin: rig_binary(),
        },
        permission_mode,
    )?;
    Ok((mcp, settings))
}

fn rig_binary() -> PathBuf {
    let Ok(path) = std::env::current_exe() else {
        return PathBuf::from("rig");
    };
    if path.is_absolute() {
        return path;
    }
    std::env::current_dir().map_or(path.clone(), |cwd| cwd.join(&path))
}

fn rig_home_of(seat_dir: &Path) -> PathBuf {
    seat_dir
        .parent()
        .and_then(Path::parent)
        .map_or_else(|| seat_dir.to_path_buf(), Path::to_path_buf)
}

async fn write_id(path: &Path, id: &str) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(path, id.as_bytes()).await?;
    Ok(())
}
