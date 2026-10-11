//! Start `rig daemon run` when nothing is listening on the socket.

use std::fs::OpenOptions;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crewmux_api::{Client, Endpoint, Healthz, daemon_socket, rig_home};

use crate::error::Error;

/// Exclusive lock on `run/daemon.start.lock` for one auto-start attempt.
///
/// `flock` releases when this process dies, so a killed CLI does not leave
/// the next start waiting on a stale file.
struct StartLock {
    file: std::fs::File,
}

impl StartLock {
    /// Take the lock, or `None` when another CLI holds it.
    fn try_acquire(home: &Path) -> Result<Option<Self>, Error> {
        let path = home.join("run/daemon.start.lock");
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        let file = crewmux_api::open_private(&path, &mut options)?;
        match rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => Ok(Some(Self { file })),
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(err) => Err(Error::Io(err.into())),
        }
    }
}

impl Drop for StartLock {
    fn drop(&mut self) {
        let _unlocked = rustix::fs::flock(&self.file, rustix::fs::FlockOperation::Unlock);
    }
}

/// Probe `/healthz` and spawn the daemon when it is down.
///
/// # Errors
///
/// Returns [`Error::Unreachable`] when the daemon does not answer after the spawn.
pub(crate) fn ensure_daemon() -> Result<Client, Error> {
    let home = rig_home();
    crewmux_api::create_private(&home)?;
    crewmux_api::create_private(&home.join("run"))?;
    let socket = daemon_socket(&home);
    if health(&socket).is_some() {
        return Ok(client(&socket));
    }
    let Some(held) = StartLock::try_acquire(&home)? else {
        wait_health(&socket)?;
        return Ok(client(&socket));
    };
    if health(&socket).is_none() {
        spawn_daemon(&home)?;
    }
    let ready = wait_health(&socket);
    drop(held);
    ready?;
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
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    let log = crewmux_api::open_private(&home.join("run/daemon.log"), &mut options)?;
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
