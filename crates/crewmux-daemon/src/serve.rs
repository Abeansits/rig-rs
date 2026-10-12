//! Process entry: multiplexer, Unix socket, and supervisors.

use std::path::Path;
use std::sync::Arc;
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
use crate::runtime::{self, DaemonLock};
use crate::state::{App, idle_exceeded, socket_name_from_env, stub_binary, stub_script_from_env};

/// Bind the daemon and serve until shutdown or idle.
///
/// # Errors
///
/// Returns [`Error`] when the database, multiplexer, or socket cannot be opened.
pub fn run() -> Result<(), Error> {
    let home = crewmux_api::rig_home();
    runtime::prepare_dirs(&home)?;
    let lock = DaemonLock::acquire(&home)?;
    let store = Arc::new(Store::open(&home.join("state/rig.db"))?);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(serve(home, store));
    drop(lock);
    result
}

async fn serve(home: std::path::PathBuf, store: Arc<Store>) -> Result<(), Error> {
    let cancel = CancellationToken::new();
    let socket_name = socket_name_from_env();
    let mux = build_mux(&socket_name, cancel.clone()).await?;
    let stub_bin = stub_binary();
    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(StubAdapter::new(
        Arc::clone(&mux),
        stub_bin.clone(),
    )));
    let loaded = {
        let store = Arc::clone(&store);
        tokio::task::spawn_blocking(move || store.read(crewmux_store::load_book))
            .await
            .map_err(crate::state::join_err)??
    };
    let app = App::assemble(
        store,
        mux,
        Arc::new(registry),
        stub_script_from_env(),
        cancel.clone(),
        home.clone(),
    );
    *app.queue.lock().await = loaded;
    let max_cursor = {
        let reading = Arc::clone(&app.store);
        tokio::task::spawn_blocking(move || reading.read(crewmux_store::max_receipt_cursor))
            .await
            .map_err(crate::state::join_err)??
    };
    app.seed_cursor(max_cursor);
    app.launch_workers().await;
    reconcile(&app).await?;
    let sock = crewmux_api::daemon_socket(&home);
    let pid = home.join("run/daemon.pid");
    let listener = {
        let sock = sock.clone();
        // `UnixListener::bind` needs the runtime context, which `spawn_blocking` drops.
        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let _entered = handle.enter();
            runtime::bind_socket(&sock)
        })
        .await
        .map_err(crate::state::join_err)??
    };
    {
        let pid = pid.clone();
        tokio::task::spawn_blocking(move || write_pid(&pid))
            .await
            .map_err(crate::state::join_err)??;
    }
    let tracker = TaskTracker::new();
    tracker.spawn(supervise(app.clone()));
    tracker.spawn(reap(app.clone()));
    tracker.spawn(on_terminate(cancel.clone()));
    let tasks = app.tasks.clone();
    axum::serve(listener, router(app))
        .with_graceful_shutdown(async move { cancel.cancelled().await })
        .await?;
    tracker.close();
    tracker.wait().await;
    tasks.close();
    tasks.wait().await;
    let sock_cleanup = sock.clone();
    let pid_cleanup = pid.clone();
    tokio::task::spawn_blocking(move || remove_runtime_files(&sock_cleanup, &pid_cleanup))
        .await
        .map_err(crate::state::join_err)??;
    Ok(())
}

async fn build_mux(socket: &str, cancel: CancellationToken) -> Result<Arc<dyn Multiplexer>, Error> {
    if fake_mux_requested() {
        let fake = Arc::new(crewmux_mux::FakeMux::with_socket(socket));
        fake.enable_stub_scripts().await;
        return Ok(fake);
    }
    let config = TmuxConfig {
        socket: socket.to_owned(),
        ..TmuxConfig::default()
    };
    let mux = TmuxMux::connect_until(config, cancel).await?;
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

fn write_pid(pid: &Path) -> Result<(), std::io::Error> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    let mut file = crewmux_api::open_private(pid, &mut options)?;
    write!(file, "{}", std::process::id())
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
