//! `rig tui`. Builds the TUI config from the environment and the integrations crate.

use std::path::PathBuf;
use std::sync::Arc;

use crewmux_api::{LauncherKind, daemon_bind_path, tui_socket_path};
use crewmux_integrations::{open_pane, open_window};
use crewmux_tui::{Config, SessionEnv, run_blocking};
use crewmux_tui::{LaunchFail, LaunchFn, LaunchRequest};

use crate::output;

/// Failure starting `rig tui`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The TUI could not take the terminal.
    #[error(transparent)]
    Tui(#[from] crewmux_tui::Error),
}

/// Run the TUI. `RIG_HOME` defaults to `./.rig` so tests never touch the home directory.
///
/// # Errors
///
/// Returns [`Error`] when the socket path is too long or the terminal cannot start.
pub fn run() -> Result<(), Error> {
    let home = match std::env::var_os("RIG_HOME") {
        Some(value) => PathBuf::from(value),
        None => PathBuf::from(".rig"),
    };
    let tmp = std::env::temp_dir();
    let uid = current_uid();
    let daemon = daemon_bind_path(&home, &tmp, uid);
    let control = tui_socket_path(&home, &tmp, uid);
    let launcher = std::env::var("RIG_LAUNCHER")
        .ok()
        .as_deref()
        .and_then(LauncherKind::parse);
    let mut env = SessionEnv::default();
    env.socket_name = std::env::var("RIG_TMUX_SOCKET").unwrap_or_else(|_| "default".to_owned());
    env.inside_tmux = inside_tmux();
    env.launcher = launcher;
    let launch = launcher.map(|kind| launch_fn(kind, herdr_socket()));
    run_blocking(Config {
        daemon_socket: daemon,
        control_socket: control,
        env,
        launch,
    })?;
    Ok(())
}

/// Print `err` so the operator sees why `rig tui` stopped.
pub fn report(err: &impl std::fmt::Display) {
    output::error_line(&err.to_string());
}

fn current_uid() -> u32 {
    let Ok(text) = std::fs::read_to_string("/proc/self/status") else {
        return 0;
    };
    text.lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(0)
}

fn inside_tmux() -> Option<String> {
    let raw = std::env::var("TMUX").ok()?;
    let path = raw.split(',').next()?;
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
}

fn herdr_socket() -> PathBuf {
    match std::env::var_os("HERDR_SOCKET") {
        Some(value) => PathBuf::from(value),
        None => PathBuf::from("/tmp/herdr.sock"),
    }
}

fn launch_fn(kind: LauncherKind, herdr: PathBuf) -> LaunchFn {
    Arc::new(move |request: LaunchRequest| {
        let herdr = herdr.clone();
        Box::pin(async move { launch(kind, &herdr, request).await })
    })
}

async fn launch(
    kind: LauncherKind,
    herdr: &std::path::Path,
    request: LaunchRequest,
) -> Result<(), LaunchFail> {
    let argv = vec![
        "tmux".to_owned(),
        "-L".to_owned(),
        request.socket.clone(),
        "attach".to_owned(),
        "-t".to_owned(),
        format!("={}", request.session),
    ];
    let result = match kind {
        LauncherKind::Herdr => open_pane(herdr, request.seat.as_str(), &argv).await,
        LauncherKind::Ghostty => open_window(&argv).await,
    };
    result.map_err(|err| LaunchFail {
        message: err.to_string(),
    })
}
