//! Process entry: directories, multiplexer, Unix socket, supervisors.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use crewmux_adapters::{AdapterRegistry, StubAdapter};
use crewmux_mux::{Multiplexer, TmuxConfig, TmuxMux};
use crewmux_store::Store;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::death::supervise;
use crate::error::Error;
use crate::http::router;
use crate::reconcile::reconcile;
use crate::state::{
    App, idle_exceeded, now_ms, socket_name_from_env, stub_binary, stub_script_from_env,
};

/// Bind the daemon and serve until shutdown or idle.
///
/// # Errors
///
/// Returns [`Error`] when the database, multiplexer, or socket cannot be opened.
pub fn run() -> Result<(), Error> {
    let home = crewmux_api::rig_home();
    prepare_dirs(&home)?;
    let store = Arc::new(Store::open(&home.join("state/rig.db"))?);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(serve(home, store))
}

fn prepare_dirs(home: &Path) -> Result<(), Error> {
    for dir in [home, &home.join("state"), &home.join("run")] {
        std::fs::create_dir_all(dir)?;
        let mut perms = std::fs::metadata(dir)?.permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(dir, perms)?;
    }
    Ok(())
}

async fn serve(home: std::path::PathBuf, store: Arc<Store>) -> Result<(), Error> {
    let socket_name = socket_name_from_env();
    let mux = build_mux(&socket_name).await?;
    let stub_bin = stub_binary();
    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(StubAdapter::new(
        Arc::clone(&mux),
        stub_bin.clone(),
    )));
    let cancel = CancellationToken::new();
    let app = App {
        store,
        mux,
        up_locks: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        stub_script: stub_script_from_env(),
        cancel: cancel.clone(),
        activity: Arc::new(AtomicU64::new(now_ms())),
        registry: Arc::new(registry),
    };
    reconcile(&app).await?;
    let sock = crewmux_api::daemon_socket(&home);
    let pid = home.join("run/daemon.pid");
    let listener = {
        let sock = sock.clone();
        let pid = pid.clone();
        tokio::task::spawn_blocking(move || bind_socket(&sock, &pid))
            .await
            .map_err(crate::state::join_err)??
    };
    {
        let pid = pid.clone();
        tokio::task::spawn_blocking(move || std::fs::write(pid, std::process::id().to_string()))
            .await
            .map_err(crate::state::join_err)??;
    }
    let tracker = TaskTracker::new();
    tracker.spawn(supervise(app.clone()));
    tracker.spawn(reap(app.clone()));
    tracker.spawn(on_terminate(cancel.clone()));
    axum::serve(listener, router(app))
        .with_graceful_shutdown(async move { cancel.cancelled().await })
        .await?;
    tracker.close();
    tracker.wait().await;
    let sock_cleanup = sock.clone();
    let pid_cleanup = pid.clone();
    tokio::task::spawn_blocking(move || remove_runtime_files(&sock_cleanup, &pid_cleanup))
        .await
        .map_err(crate::state::join_err)??;
    Ok(())
}

async fn build_mux(socket: &str) -> Result<Arc<dyn Multiplexer>, Error> {
    if fake_mux_requested() {
        let fake = Arc::new(crewmux_mux::FakeMux::with_socket(socket));
        fake.enable_stub_scripts().await;
        return Ok(fake);
    }
    let config = TmuxConfig {
        socket: socket.to_owned(),
        ..TmuxConfig::default()
    };
    let mux = TmuxMux::connect(config).await?;
    Ok(Arc::new(mux))
}

fn fake_mux_requested() -> bool {
    // Debug-only so a release binary cannot be pointed at the in-memory multiplexer.
    #[cfg(all(feature = "fake-mux", debug_assertions))]
    {
        std::env::var("RIG_FAKE_MUX").ok().as_deref() == Some("1")
    }
    #[cfg(not(all(feature = "fake-mux", debug_assertions)))]
    {
        false
    }
}

fn bind_socket(path: &Path, pid_path: &Path) -> Result<tokio::net::UnixListener, Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        if private_dir(parent) {
            let mut perms = std::fs::metadata(parent)?.permissions();
            perms.set_mode(0o700);
            std::fs::set_permissions(parent, perms)?;
        }
    }
    if daemon_is_live(path, pid_path) {
        return Err(Error::Message(format!(
            "daemon is already running at {}",
            path.display()
        )));
    }
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    let listener = tokio::net::UnixListener::bind(path)?;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(path, perms)?;
    Ok(listener)
}

fn private_dir(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name == "run" || name == "state" || name.starts_with("rig-")
}

fn daemon_is_live(socket: &Path, pid_path: &Path) -> bool {
    if std::os::unix::net::UnixStream::connect(socket).is_ok() {
        return true;
    }
    let Ok(text) = std::fs::read_to_string(pid_path) else {
        return false;
    };
    let Ok(pid) = text.trim().parse::<u32>() else {
        return false;
    };
    process_alive(pid)
}

fn process_alive(pid: u32) -> bool {
    if pid <= 1 {
        return false;
    }
    let Ok(status) = std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    else {
        return false;
    };
    status.success()
}

fn remove_runtime_files(socket: &Path, pid: &Path) -> Result<(), Error> {
    remove_if_present(socket)?;
    remove_if_present(pid)?;
    Ok(())
}

fn remove_if_present(path: &Path) -> Result<(), Error> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

async fn reap(app: App) {
    loop {
        tokio::select! {
            () = app.cancel.cancelled() => break,
            () = tokio::time::sleep(Duration::from_secs(5)) => {
                if idle_exceeded(&app.activity) {
                    app.cancel.cancel();
                    break;
                }
            }
        }
    }
}

async fn on_terminate(cancel: CancellationToken) {
    let mut terminate =
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(signal) => signal,
            Err(err) => {
                tracing::warn!(error = %err, "sigterm handler");
                return;
            }
        };
    tokio::select! {
        () = cancel.cancelled() => {}
        _ = terminate.recv() => cancel.cancel(),
    }
}

#[cfg(test)]
mod bind_tests {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;

    use super::bind_socket;

    fn scratch(label: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("shared-{label}-{}-{}", std::process::id(), label));
        drop(std::fs::remove_dir_all(&path));
        std::fs::create_dir_all(&path).expect("scratch");
        let mut perms = std::fs::metadata(&path).expect("meta").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("mode");
        path
    }

    #[tokio::test]
    async fn shared_parent_keeps_its_mode() {
        let root = scratch("mode");
        let private = root.join("rig-test");
        let sock = private.join("daemon.sock");
        let pid = root.join("no-such.pid");
        let listener = bind_socket(&sock, &pid).expect("bind");
        drop(listener);
        let shared = std::fs::metadata(&root)
            .expect("shared")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(shared, 0o755, "refusing to chmod a shared directory");
        let mode = std::fs::metadata(&private)
            .expect("private")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
        drop(std::fs::remove_dir_all(&root));
    }

    #[test]
    fn live_socket_is_not_unlinked() {
        let root = scratch("live");
        let private = root.join("run");
        std::fs::create_dir_all(&private).expect("run");
        let sock = private.join("daemon.sock");
        let held = UnixListener::bind(&sock).expect("listen");
        let err = bind_socket(&sock, &root.join("daemon.pid")).expect_err("live");
        assert!(err.to_string().contains("already running"), "{err}");
        assert!(sock.exists());
        drop(held);
        drop(std::fs::remove_dir_all(&root));
    }
}
