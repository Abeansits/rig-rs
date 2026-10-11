//! Start `rig daemon run` when nothing is listening on the socket.

use std::fs::OpenOptions;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crewmux_api::{Client, Endpoint, Healthz, daemon_socket, rig_home};

use crate::error::Error;

/// Probe `/healthz` and spawn the daemon when it is down.
///
/// # Errors
///
/// Returns [`Error::Unreachable`] when the daemon does not answer after the spawn.
pub(crate) fn ensure_daemon() -> Result<Client, Error> {
    let home = rig_home();
    std::fs::create_dir_all(home.join("run"))?;
    let socket = daemon_socket(&home);
    if health(&socket).is_some() {
        return Ok(client(&socket));
    }
    let lock_path = home.join("run/daemon.start.lock");
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
    {
        Ok(_lock) => {
            if let Err(err) = spawn_daemon(&home) {
                drop(std::fs::remove_file(&lock_path));
                return Err(err);
            }
            let ready = wait_health(&socket);
            drop(std::fs::remove_file(&lock_path));
            ready?;
        }
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            wait_health(&socket)?;
        }
        Err(err) => return Err(err.into()),
    }
    Ok(client(&socket))
}

fn client(socket: &Path) -> Client {
    Client::over_uds(socket).with_timeout(Duration::from_secs(120))
}

fn health(socket: &Path) -> Option<crewmux_api::Response> {
    let response = Client::over_uds(socket)
        .with_timeout(Duration::from_millis(200))
        .request(Healthz::METHOD, Healthz::PATH, None)
        .ok()?;
    if response.status == 200 {
        Some(response)
    } else {
        None
    }
}

fn wait_health(socket: &Path) -> Result<(), Error> {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if health(socket).is_some() {
            return Ok(());
        }
        std::thread::park_timeout(Duration::from_millis(50));
    }
    Err(Error::Unreachable(
        "daemon did not answer /healthz after auto-start".to_owned(),
    ))
}

fn spawn_daemon(home: &Path) -> Result<(), Error> {
    let exe = std::env::current_exe()?;
    let log_path = home.join("run/daemon.log");
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let err_log = log.try_clone()?;
    let mut command = Command::new(exe);
    command
        .arg("daemon")
        .arg("run")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err_log))
        .env("RIG_HOME", home)
        .process_group(0);
    command
        .spawn()
        .map_err(|err| Error::Unreachable(format!("failed to spawn the daemon: {err}")))?;
    Ok(())
}
