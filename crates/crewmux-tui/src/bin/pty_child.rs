//! Child process for the real-tmux pty round trip. Built only with `--features tmux-it`.

use std::path::PathBuf;
use std::process::ExitCode;

use crewmux_tui::{Config, SessionEnv, run_blocking};

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

fn main() -> ExitCode {
    let home = match std::env::var_os("RIG_HOME") {
        Some(value) => PathBuf::from(value),
        None => PathBuf::from(".rig"),
    };
    let tmp = std::env::temp_dir();
    let uid = current_uid();
    let daemon = crewmux_api::daemon_bind_path(&home, &tmp, uid);
    let control = crewmux_api::tui_socket_path(&home, &tmp, uid);
    let mut env = SessionEnv::default();
    env.socket_name = std::env::var("RIG_TMUX_SOCKET").unwrap_or_else(|_| "rig-rs".to_owned());
    match run_blocking(Config {
        daemon_socket: daemon,
        control_socket: control,
        env,
        launch: None,
    }) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::from(1),
    }
}
