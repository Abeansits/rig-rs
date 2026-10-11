//! Process entry: directories, multiplexer, Unix socket, supervisors.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use crewmux_adapters::{AdapterRegistry, StubAdapter};
use crewmux_mux::{FakeMux, Multiplexer, TmuxConfig, TmuxMux};
use crewmux_store::Store;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::death::supervise;
use crate::error::Error;
use crate::http::router;
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
        socket_name,
        stub_script: stub_script_from_env(),
        cancel: cancel.clone(),
        activity: Arc::new(AtomicU64::new(now_ms())),
        registry: Arc::new(registry),
    };
    let sock = crewmux_api::daemon_socket(&home);
    let listener = tokio::task::spawn_blocking(move || bind_socket(&sock))
        .await
        .map_err(crate::state::join_err)??;
    let pid = home.join("run/daemon.pid");
    tokio::task::spawn_blocking(move || std::fs::write(pid, std::process::id().to_string()))
        .await
        .map_err(crate::state::join_err)??;
    let tracker = TaskTracker::new();
    tracker.spawn(supervise(app.clone()));
    tracker.spawn(reap(app.clone()));
    axum::serve(listener, router(app))
        .with_graceful_shutdown(async move { cancel.cancelled().await })
        .await?;
    tracker.close();
    tracker.wait().await;
    Ok(())
}

async fn build_mux(socket: &str) -> Result<Arc<dyn Multiplexer>, Error> {
    if std::env::var("RIG_FAKE_MUX").ok().as_deref() == Some("1") {
        let fake = Arc::new(FakeMux::with_socket(socket));
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

fn bind_socket(path: &Path) -> Result<tokio::net::UnixListener, Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        let mut perms = std::fs::metadata(parent)?.permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(parent, perms)?;
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
