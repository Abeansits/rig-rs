//! Live [`TmuxMux`](crate::TmuxMux) driver.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crewmux_types::parse_version;
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::argv::{
    armed_session_args, bell_window_args, canonical_session_name, capture_args, cursor_args,
    has_session_args, is_pane_id, link_into_control_args, list_clients_args, list_panes_args,
    manual_size_args, pane_mode_args, pane_target, parse_bell_id, reject_global_window_size,
    resize_args, send_keys_args, set_option_args,
};
use crate::clients::{filter_human_clients, last_human_keystroke};
use crate::control::{ControlCtx, run_control};
use crate::control_child::ControlSlot;
use crate::error::{MuxError, ensure_supported};
use crate::exec::{ProcessExec, TmuxExec, is_absence, resolve_tmux_binary};
use crate::hub::Hub;
use crate::parse::{parse_clients, parse_created, parse_cursor, parse_panes};
use crate::paste::perform_paste;
use crate::types::{
    Capture, CaptureRange, ClientInfo, DeathStream, GuardInputs, MuxEvent, OptScope, OutputStream,
    PaneInfo, PaneRef, PasteOpts, PasteReceipt, SessionSpec,
};
use crate::{Key, Multiplexer};

/// How to reach a tmux server.
#[derive(Clone, Debug)]
pub struct TmuxConfig {
    /// tmux binary. Defaults to `tmux` on `PATH`.
    pub binary: PathBuf,
    /// `-L` socket name. Defaults to `rig-rs`.
    pub socket: String,
    /// When true, the control client sends `refresh-client -f pause-after`.
    ///
    /// Default is false. The flag is stored and not otherwise tested.
    pub pause_after: bool,
}

impl Default for TmuxConfig {
    fn default() -> Self {
        Self {
            binary: PathBuf::from("tmux"),
            socket: "rig-rs".to_owned(),
            pause_after: false,
        }
    }
}

/// A tmux server on one private socket, with a single control-mode client.
pub struct TmuxMux {
    exec: Arc<dyn TmuxExec>,
    socket: String,
    binary: String,
    socket_path: String,
    bell: String,
    hub: Arc<tokio::sync::Mutex<Hub>>,
    events: broadcast::Sender<MuxEvent>,
    own_write: watch::Sender<Option<Instant>>,
    cancel: CancellationToken,
    child: Arc<ControlSlot>,
    tracker: TaskTracker,
    control: JoinHandle<()>,
}

impl Drop for TmuxMux {
    fn drop(&mut self) {
        self.cancel.cancel();
        let child = Arc::clone(&self.child);
        let binary = self.binary.clone();
        let socket = self.socket.clone();
        let reaper = std::thread::spawn(move || {
            child.kill_and_wait();
            // The server is forked from the control client and reparented to
            // init, so killing the client leaves it running. This mux started
            // that client, including when a parent process chose the socket
            // name (`rig-rs-<their pid>-…`). Tests still refuse to kill a
            // socket they do not own; see [`socket_owned_by`].
            //
            // `kill -9` of the daemon skips this Drop. That server stays up
            // until the next daemon starts: `new-session -A` attaches to it
            // instead of leaving a second server, and the next graceful
            // shutdown is what finally kills it.
            kill_server(&binary, &socket);
        });
        drop(reaper.join());
        self.control.abort();
        self.tracker.close();
    }
}

/// `rig-rs-<pid>` or `rig-rs-<pid>-<suffix>` for `pid`.
///
/// The bare name `rig-rs` is the shared default and is never owned.
#[must_use]
pub fn socket_owned_by(socket: &str, pid: u32) -> bool {
    let Some(rest) = socket.strip_prefix("rig-rs-") else {
        return false;
    };
    let own = pid.to_string();
    match rest.split_once('-') {
        Some((head, tail)) => head == own && !tail.is_empty(),
        None => rest == own,
    }
}

/// Default tmux socket file for `-L name`.
///
/// Commands clear `TMUX_TMPDIR`, so the server uses `/tmp/tmux-<uid>/<name>`.
#[must_use]
pub fn socket_path(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("/tmp/tmux-{}", rustix::process::getuid().as_raw())).join(name)
}

fn kill_server(binary: &str, socket: &str) {
    let mut command = std::process::Command::new(binary);
    command
        .args(["-L", socket, "-f", "/dev/null", "kill-server"])
        .env_remove("TMUX")
        .env_remove("TMUX_TMPDIR")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    drop(command.status());
    // A server that has already exited leaves the socket file behind.
    drop(std::fs::remove_file(socket_path(socket)));
}

impl TmuxMux {
    /// Check the tmux version, then start the control client.
    ///
    /// A version older than 3.3 returns [`MuxError::TmuxTooOld`] and does not spawn.
    pub async fn connect(config: TmuxConfig) -> Result<Self, MuxError> {
        Self::connect_until(config, CancellationToken::new()).await
    }

    /// [`connect`](Self::connect), stopping the control client when `cancel` fires.
    ///
    /// The daemon passes its shutdown token so a dying server is not replaced
    /// with `new-session -A` while the process is exiting.
    pub async fn connect_until(
        config: TmuxConfig,
        cancel: CancellationToken,
    ) -> Result<Self, MuxError> {
        let binary = resolve_tmux_binary(&config.binary).await;
        let exec: Arc<dyn TmuxExec> =
            Arc::new(ProcessExec::new(binary.clone(), config.socket.clone()));
        let config = TmuxConfig {
            binary,
            socket: config.socket,
            pause_after: config.pause_after,
        };
        Self::connect_with_exec(config, exec, cancel).await
    }

    /// Same as [`connect`](Self::connect), with a caller-supplied exec implementation.
    ///
    /// The version probe goes through `exec`. The control client is still a process
    /// started from [`TmuxConfig::binary`], and only after the version is accepted.
    pub async fn connect_with_exec(
        config: TmuxConfig,
        exec: Arc<dyn TmuxExec>,
        cancel: CancellationToken,
    ) -> Result<Self, MuxError> {
        ensure_version(exec.as_ref()).await?;
        let binary = config
            .binary
            .to_str()
            .ok_or_else(|| MuxError::Control("tmux path is not utf-8".into()))?;
        let binary = binary.to_owned();
        let tracker = TaskTracker::new();
        let (boot_tx, boot_rx) = tokio::sync::oneshot::channel();
        let (events, _) = broadcast::channel(2048);
        let (own_write, _) = watch::channel(None);
        let (bell_tx, bell_rx) = watch::channel(None);
        let hub = Arc::new(tokio::sync::Mutex::new(Hub::default()));
        let child = Arc::new(ControlSlot::new());
        let ctx = ControlCtx {
            binary: config.binary.clone(),
            socket: config.socket.clone(),
            pause_after: config.pause_after,
            exec: Arc::clone(&exec),
            hub: Arc::clone(&hub),
            events: events.clone(),
            cancel: cancel.clone(),
            child: Arc::clone(&child),
            boot_tx: Some(boot_tx),
            bell: bell_rx,
        };
        let nudge_rx = events.subscribe();
        let control = tracker.spawn(run_control(ctx));
        if let Err(err) = wait_ready(boot_rx).await {
            fail_connect(&cancel, &child, &control, &binary, &config.socket).await;
            return Err(err);
        }
        tracker.spawn(crate::reap::nudge(
            Arc::clone(&exec),
            config.socket.clone(),
            nudge_rx,
            cancel.clone(),
        ));
        let bell = match open_bell(exec.as_ref()).await {
            Ok(bell) => bell,
            Err(err) => {
                fail_connect(&cancel, &child, &control, &binary, &config.socket).await;
                return Err(err);
            }
        };
        bell_tx.send_replace(Some(bell.clone()));
        let socket_path = match read_socket_path(exec.as_ref()).await {
            Ok(path) => path,
            Err(err) => {
                fail_connect(&cancel, &child, &control, &binary, &config.socket).await;
                return Err(err);
            }
        };
        Ok(Self {
            exec,
            socket: config.socket,
            binary,
            socket_path,
            bell,
            hub,
            events,
            own_write,
            cancel,
            child,
            tracker,
            control,
        })
    }

    /// Socket name passed as `-L`.
    #[must_use]
    pub fn socket(&self) -> &str {
        &self.socket
    }
}

async fn read_socket_path(exec: &dyn TmuxExec) -> Result<String, MuxError> {
    let out = exec
        .run(&[
            "display-message".to_owned(),
            "-p".to_owned(),
            "#{socket_path}".to_owned(),
        ])
        .await?;
    let path = out.stdout.trim();
    if path.is_empty() {
        return Err(MuxError::Control(
            "tmux did not report a socket path".to_owned(),
        ));
    }
    Ok(path.to_owned())
}

async fn open_bell(exec: &dyn TmuxExec) -> Result<String, MuxError> {
    let out = exec.run(&bell_window_args()).await?;
    parse_bell_id(&out.stdout)
}

async fn ensure_version(exec: &dyn TmuxExec) -> Result<(), MuxError> {
    let out = exec.run(&["-V".to_owned()]).await?;
    let found = parse_version(&out.stdout).map_err(MuxError::Control)?;
    ensure_supported(&found)
}

async fn fail_connect(
    cancel: &CancellationToken,
    child: &Arc<ControlSlot>,
    control: &JoinHandle<()>,
    binary: &str,
    socket: &str,
) {
    cancel.cancel();
    let slot = Arc::clone(child);
    let binary = binary.to_owned();
    let socket = socket.to_owned();
    drop(
        tokio::task::spawn_blocking(move || {
            slot.kill_and_wait();
            kill_server(&binary, &socket);
        })
        .await,
    );
    control.abort();
}

async fn wait_ready(
    boot_rx: tokio::sync::oneshot::Receiver<Result<(), MuxError>>,
) -> Result<(), MuxError> {
    match tokio::time::timeout(Duration::from_secs(8), boot_rx).await {
        Ok(Ok(Ok(()))) => Ok(()),
        Ok(Ok(Err(err))) => Err(err),
        Ok(Err(_closed)) => Err(MuxError::Control(
            "control task ended before it was ready".into(),
        )),
        Err(_elapsed) => Err(MuxError::Control(
            "timed out starting the control client".into(),
        )),
    }
}

fn touch_write(own: &watch::Sender<Option<Instant>>) {
    own.send_replace(Some(Instant::now()));
}

fn require_pane_id(pane: &PaneRef) -> Result<String, MuxError> {
    pane.pane_id
        .clone()
        .filter(|id| is_pane_id(id))
        .ok_or_else(|| MuxError::Control("pane id is required".into()))
}

#[async_trait::async_trait]
impl Multiplexer for TmuxMux {
    async fn create_session(&self, spec: &SessionSpec) -> Result<PaneRef, MuxError> {
        let created = self
            .exec
            .run(&armed_session_args(
                spec,
                &self.binary,
                &self.socket,
                &self.bell,
            )?)
            .await?;
        let (pane_id, session) = parse_created(&created.stdout)?;
        self.exec.run(&manual_size_args(&session)).await?;
        self.exec
            .run(&resize_args(
                &session,
                spec.geometry.cols,
                spec.geometry.rows,
            ))
            .await?;
        self.exec.run(&link_into_control_args(&session)).await?;
        let panes = self.list_panes().await?;
        self.hub.lock().await.note_panes(&panes);
        Ok(PaneRef {
            session,
            window: Some(0),
            pane: Some(0),
            pane_id: Some(pane_id),
        })
    }

    async fn has_session(&self, name: &str) -> Result<bool, MuxError> {
        match self.exec.run(&has_session_args(name)).await {
            Ok(_) => Ok(true),
            Err(MuxError::NotFound(_)) => Ok(false),
            Err(MuxError::Exec { stderr, .. }) if is_absence(&stderr) => Ok(false),
            Err(err) => Err(err),
        }
    }

    async fn kill_session(&self, name: &str) -> Result<(), MuxError> {
        crate::session_kill::unlink_and_kill(self.exec.as_ref(), name).await
    }

    async fn list_panes(&self) -> Result<Vec<PaneInfo>, MuxError> {
        let out = self.exec.run(&list_panes_args()).await?;
        parse_panes(&out.stdout)
    }

    async fn send_keys(&self, pane: &PaneRef, keys: &[Key]) -> Result<(), MuxError> {
        for batch in send_keys_args(&pane_target(pane), keys)? {
            self.exec.run(&batch).await?;
        }
        touch_write(&self.own_write);
        Ok(())
    }

    async fn paste(
        &self,
        pane: &PaneRef,
        text: &str,
        opts: PasteOpts,
    ) -> Result<PasteReceipt, MuxError> {
        let receipt = perform_paste(self.exec.as_ref(), &pane_target(pane), text, opts).await?;
        touch_write(&self.own_write);
        Ok(receipt)
    }

    async fn capture(&self, pane: &PaneRef, range: CaptureRange) -> Result<Capture, MuxError> {
        let target = pane_target(pane);
        let text = self.exec.run(&capture_args(&target, range)).await?;
        let cursor = self.exec.run(&cursor_args(&target)).await?;
        Ok(Capture::new(
            trim_capture(&text.stdout),
            parse_cursor(&cursor.stdout)?,
        ))
    }

    async fn subscribe_output(&self, pane: &PaneRef) -> Result<OutputStream, MuxError> {
        let pane_id = require_pane_id(pane)?;
        Ok(self.hub.lock().await.subscribe(&pane_id))
    }

    async fn set_option(&self, scope: OptScope, key: &str, value: &str) -> Result<(), MuxError> {
        reject_global_window_size(&scope, key)?;
        self.exec
            .run(&set_option_args(&scope, key, value))
            .await
            .map(|_| ())
    }

    async fn human_clients(&self, session: &str) -> Result<Vec<ClientInfo>, MuxError> {
        let out = self.exec.run(&list_clients_args()).await?;
        let clients = parse_clients(&out.stdout)?;
        Ok(filter_human_clients(
            &clients,
            &canonical_session_name(session),
        ))
    }

    async fn pane_in_mode(&self, pane: &PaneRef) -> Result<bool, MuxError> {
        let out = self.exec.run(&pane_mode_args(&pane_target(pane))).await?;
        Ok(out.stdout.trim() == "1")
    }

    async fn guard_inputs(&self, pane: &PaneRef, tail_lines: u16) -> Result<GuardInputs, MuxError> {
        let session = canonical_session_name(&pane.session);
        let human_clients = self.human_clients(&session).await?;
        let last_human_keystroke_tmux = last_human_keystroke(&human_clients, &session);
        Ok(GuardInputs {
            human_clients,
            last_human_keystroke_tmux,
            last_human_keystroke_tap: None,
            pane_in_mode: self.pane_in_mode(pane).await?,
            tail: self
                .capture(pane, CaptureRange::LastLines(tail_lines))
                .await?,
            last_own_write: *self.own_write.borrow(),
        })
    }

    fn death_events(&self) -> DeathStream {
        DeathStream {
            rx: self.events.subscribe(),
        }
    }

    fn events(&self) -> broadcast::Receiver<MuxEvent> {
        self.events.subscribe()
    }

    fn attach_program(&self) -> (String, String) {
        (self.binary.clone(), self.socket_path.clone())
    }
}

fn trim_capture(stdout: &str) -> String {
    stdout.trim_end_matches(['\n', '\r']).to_owned()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;

    use tokio_util::sync::CancellationToken;

    use super::{TmuxConfig, TmuxMux};
    use crate::error::MuxError;
    use crate::exec::{ExecOutput, TmuxExec};

    struct VersionOnly {
        stdout: String,
        calls: AtomicUsize,
    }

    #[async_trait]
    impl TmuxExec for VersionOnly {
        async fn run(&self, args: &[String]) -> Result<ExecOutput, MuxError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if args.first().map(String::as_str) == Some("-V") {
                return Ok(ExecOutput {
                    stdout: self.stdout.clone(),
                    stderr: String::new(),
                });
            }
            Err(MuxError::Control(format!("unexpected argv {args:?}")))
        }
    }

    #[tokio::test]
    async fn connect_rejects_tmux_older_than_3_3_without_a_control_client() {
        let probe = Arc::new(VersionOnly {
            stdout: "tmux 3.2a\n".into(),
            calls: AtomicUsize::new(0),
        });
        let connected = TmuxMux::connect_with_exec(
            TmuxConfig::default(),
            erase(Arc::clone(&probe)),
            CancellationToken::new(),
        )
        .await;
        let Err(err) = connected else {
            panic!("old tmux was accepted");
        };
        assert!(matches!(err, MuxError::TmuxTooOld { .. }), "{err}");
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    }

    fn erase(exec: Arc<VersionOnly>) -> Arc<dyn TmuxExec> {
        exec
    }

    #[test]
    fn only_a_pid_prefixed_socket_is_owned() {
        assert!(super::socket_owned_by("rig-rs-42", 42));
        assert!(super::socket_owned_by("rig-rs-42-7", 42));
        assert!(super::socket_owned_by("rig-rs-42-e2e", 42));
        assert!(!super::socket_owned_by("rig-rs", 42));
        assert!(!super::socket_owned_by("rig-rs-", 42));
        assert!(!super::socket_owned_by("rig-rs-42-", 42));
        assert!(!super::socket_owned_by("rig-rs-420-1", 42));
        assert!(!super::socket_owned_by("rig-rs-7", 42));
        assert!(!super::socket_owned_by("other-42", 42));
    }
}
