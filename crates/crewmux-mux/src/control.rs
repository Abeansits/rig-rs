//! One control-mode client on `crewmux-control` (no `-d`). Linked windows
//! deliver `%output`. Format strings on this channel are single-quoted.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::{broadcast, oneshot, watch};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::argv::{backstop_command, dead_status_args, delete_buffer_args, list_panes_args};
use crate::control_child::{ControlSlot, SpawnError};
use crate::control_line::{Action, LineBuf, classify, pane_from_dead_buffer};
use crate::decode::OutputDecoder;
use crate::error::MuxError;
use crate::exec::TmuxExec;
use crate::hub::{Hub, push_output};
use crate::parse::{parse_dead_status, parse_panes};
use crate::types::MuxEvent;

pub(crate) struct ControlCtx {
    pub(crate) binary: PathBuf,
    pub(crate) socket: String,
    pub(crate) pause_after: bool,
    pub(crate) exec: Arc<dyn TmuxExec>,
    pub(crate) hub: Arc<tokio::sync::Mutex<Hub>>,
    pub(crate) events: broadcast::Sender<MuxEvent>,
    pub(crate) cancel: CancellationToken,
    pub(crate) child: Arc<ControlSlot>,
    pub(crate) boot_tx: Option<oneshot::Sender<Result<(), MuxError>>>,
    /// Bell window id (`@N`), published after the control client is ready.
    pub(crate) bell: watch::Receiver<Option<String>>,
}

enum Drive {
    Cancelled,
    Exited,
    Failed(MuxError),
}

enum Step {
    Continue,
    Stop,
}

struct Boot {
    tx: Option<oneshot::Sender<Result<(), MuxError>>>,
}

struct DriveState<'a> {
    ctx: &'a ControlCtx,
    boot: &'a mut Boot,
    reconnecting: bool,
    ready: bool,
    stdin: &'a mut ChildStdin,
    lines: LineBuf,
    decoders: HashMap<String, OutputDecoder>,
    pending: HashMap<String, Vec<u8>>,
    confirms: &'a mut JoinSet<()>,
}

/// Run the control client until cancelled.
///
/// A dead server is restarted only while shutdown has not cancelled `ctx`.
/// `new-session -A` would otherwise fork a new server during teardown.
pub(crate) async fn run_control(mut ctx: ControlCtx) {
    let mut boot = Boot {
        tx: ctx.boot_tx.take(),
    };
    let mut delay = Duration::from_millis(50);
    let mut reconnecting = false;
    loop {
        if ctx.cancel.is_cancelled() {
            report_boot(
                &mut boot,
                Err(MuxError::Control(
                    "cancelled before the control client was ready".into(),
                )),
            );
            break;
        }
        match drive_client(&ctx, &mut boot, reconnecting).await {
            Drive::Cancelled => break,
            Drive::Exited => {
                drop(ctx.events.send(MuxEvent::ServerExited));
                reconnecting = true;
            }
            Drive::Failed(err) => {
                if boot.tx.is_some() {
                    report_boot(&mut boot, Err(err));
                    break;
                }
                tracing::warn!(error = %err, socket = %ctx.socket, "control client failed");
                reconnecting = true;
            }
        }
        if ctx.cancel.is_cancelled() {
            break;
        }
        tokio::select! {
            () = ctx.cancel.cancelled() => break,
            () = tokio::time::sleep(delay) => {}
        }
        delay = (delay * 2).min(Duration::from_secs(1));
    }
}

fn report_boot(boot: &mut Boot, result: Result<(), MuxError>) {
    if let Some(tx) = boot.tx.take() {
        drop(tx.send(result));
    }
}

async fn drive_client(ctx: &ControlCtx, boot: &mut Boot, reconnecting: bool) -> Drive {
    let mut child = match ctx.child.spawn(&ctx.binary, &ctx.socket, &ctx.cancel).await {
        Ok(child) => child,
        Err(SpawnError::Cancelled) => return Drive::Cancelled,
        Err(SpawnError::Io(err)) => return Drive::Failed(err.into()),
    };
    let pid = child.id();
    let Some(mut stdin) = child.stdin.take() else {
        return Drive::Failed(MuxError::Control("control client has no stdin".into()));
    };
    let Some(mut stdout) = child.stdout.take() else {
        return Drive::Failed(MuxError::Control("control client has no stdout".into()));
    };
    let mut confirms = JoinSet::new();
    let mut state = DriveState {
        ctx,
        boot,
        reconnecting,
        ready: false,
        stdin: &mut stdin,
        lines: LineBuf::default(),
        decoders: HashMap::new(),
        pending: HashMap::new(),
        confirms: &mut confirms,
    };
    let outcome = read_client(&mut state, &mut stdout).await;
    drop(state);
    confirms.abort_all();
    drop(stdin);
    reap(child).await;
    if let Some(pid) = pid {
        ctx.child.clear(pid).await;
    }
    outcome
}

enum ReadAction {
    Continue,
    Stop(Drive),
}

fn drive_unless_ready(ready: bool, err: MuxError) -> Drive {
    if ready {
        Drive::Exited
    } else {
        Drive::Failed(err)
    }
}

fn note_confirm(joined: Result<(), tokio::task::JoinError>) {
    if let Err(err) = joined {
        tracing::debug!(error = %err, "death confirm ended");
    }
}

async fn consume_read(
    state: &mut DriveState<'_>,
    buf: &mut [u8],
    read: std::io::Result<usize>,
) -> ReadAction {
    let n = match read {
        Ok(n) => n,
        Err(err) => return ReadAction::Stop(drive_unless_ready(state.ready, err.into())),
    };
    if n == 0 {
        return ReadAction::Stop(drive_unless_ready(
            state.ready,
            MuxError::Control("control client closed before it was ready".into()),
        ));
    }
    let owned = buf.get(..n).unwrap_or_default().to_vec();
    match dispatch_chunk(state, &owned).await {
        Ok(Step::Continue) => ReadAction::Continue,
        Ok(Step::Stop) => ReadAction::Stop(drive_unless_ready(
            state.ready,
            MuxError::Control("control client exited before it was ready".into()),
        )),
        Err(err) if state.ready => {
            tracing::warn!(error = %err, "control line");
            ReadAction::Continue
        }
        Err(err) => ReadAction::Stop(Drive::Failed(err)),
    }
}

async fn read_client(state: &mut DriveState<'_>, stdout: &mut ChildStdout) -> Drive {
    let mut buf = vec![0_u8; 8192];
    loop {
        tokio::select! {
            biased;
            () = state.ctx.cancel.cancelled() => return Drive::Cancelled,
            Some(joined) = state.confirms.join_next(), if !state.confirms.is_empty() => {
                note_confirm(joined);
            }
            read = stdout.read(&mut buf) => {
                if let ReadAction::Stop(drive) = consume_read(state, &mut buf, read).await {
                    return drive;
                }
            }
        }
    }
}

async fn reap(mut child: Child) {
    drop(tokio::time::timeout(Duration::from_secs(2), child.wait()).await);
}

async fn dispatch_chunk(state: &mut DriveState<'_>, chunk: &[u8]) -> Result<Step, MuxError> {
    let lines = state.lines.push(chunk);
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Step::Stop = dispatch_line(state, &line).await? {
            flush_pending(state).await;
            return Ok(Step::Stop);
        }
    }
    flush_pending(state).await;
    Ok(Step::Continue)
}

async fn dispatch_line(state: &mut DriveState<'_>, line: &[u8]) -> Result<Step, MuxError> {
    match classify(line) {
        Action::Output { pane_id, data } => on_output(state, &pane_id, &data).await,
        Action::WindowClose { window_id } => emit_window_close(state.ctx, &window_id).await,
        Action::PasteBuffer { name } => {
            if let Some(pane_id) = pane_from_dead_buffer(&name) {
                spawn_confirm(state, pane_id, Some(name));
            }
        }
        Action::WindowRenamed { window_id, pane_id } => {
            if bell_matches(&state.ctx.bell, &window_id) {
                let buffer = format!("rig-dead-{pane_id}");
                spawn_confirm(state, pane_id, Some(buffer));
            }
        }
        Action::Subscription { pane_id } => spawn_confirm(state, pane_id, None),
        Action::SessionsChanged => {
            drop(state.ctx.events.send(MuxEvent::SessionsChanged));
        }
        Action::SessionChanged => boot_on_session(state).await?,
        Action::Exit => return Ok(Step::Stop),
        Action::Ignore => {}
    }
    Ok(Step::Continue)
}

async fn on_output(state: &mut DriveState<'_>, pane_id: &str, data: &[u8]) {
    let decoded = {
        let decoder = state.decoders.entry(pane_id.to_owned()).or_default();
        let pushed = decoder.push(data);
        if !decoder.is_idle() {
            tracing::debug!(pane_id, "partial output escape");
        }
        pushed
    };
    match decoded {
        Ok(bytes) if !bytes.is_empty() => {
            state
                .pending
                .entry(pane_id.to_owned())
                .or_default()
                .extend_from_slice(&bytes);
            loop {
                let chunk = {
                    let Some(buf) = state.pending.get_mut(pane_id) else {
                        break;
                    };
                    if buf.len() < crate::hub::CHUNK {
                        break;
                    }
                    buf.drain(..crate::hub::CHUNK).collect::<Vec<u8>>()
                };
                push_output(&state.ctx.hub, &state.ctx.events, pane_id, &chunk).await;
            }
        }
        Ok(_) => {}
        Err(err) => tracing::debug!(error = %err, pane_id, "output decode"),
    }
}

async fn flush_pending(state: &mut DriveState<'_>) {
    let pending = std::mem::take(&mut state.pending);
    for (pane_id, bytes) in pending {
        if !bytes.is_empty() {
            push_output(&state.ctx.hub, &state.ctx.events, &pane_id, &bytes).await;
        }
    }
}

async fn boot_on_session(state: &mut DriveState<'_>) -> Result<(), MuxError> {
    if state.ready {
        return Ok(());
    }
    finish_boot(state).await?;
    state.ready = true;
    if state.reconnecting {
        drop(state.ctx.events.send(MuxEvent::Reconnected));
    }
    report_boot(state.boot, Ok(()));
    tracing::info!(socket = %state.ctx.socket, reconnecting = state.reconnecting, "control client ready");
    Ok(())
}

async fn finish_boot(state: &mut DriveState<'_>) -> Result<(), MuxError> {
    // Not `set-option -g window-size`: on tmux 3.5a that plus a control client
    // makes the next `new-session` kill the server. Each session sets it itself.
    let listed = state.ctx.exec.run(&list_panes_args()).await?;
    let panes = parse_panes(&listed.stdout)?;
    state.ctx.hub.lock().await.note_panes(&panes);
    let mut command = backstop_command();
    command.push('\n');
    state.stdin.write_all(command.as_bytes()).await?;
    if state.ctx.pause_after {
        state
            .stdin
            .write_all(b"refresh-client -f pause-after\n")
            .await?;
    }
    state.stdin.flush().await?;
    Ok(())
}

fn bell_matches(bell: &watch::Receiver<Option<String>>, window_id: &str) -> bool {
    bell.borrow().as_deref() == Some(window_id)
}

fn spawn_confirm(state: &mut DriveState<'_>, pane_id: String, buffer: Option<String>) {
    let exec = Arc::clone(&state.ctx.exec);
    let hub = Arc::clone(&state.ctx.hub);
    let events = state.ctx.events.clone();
    let cancel = state.ctx.cancel.clone();
    state.confirms.spawn(async move {
        tokio::select! {
            () = cancel.cancelled() => {}
            () = confirm_death(exec, hub, events, pane_id, buffer) => {}
        }
    });
}

async fn confirm_death(
    exec: Arc<dyn TmuxExec>,
    hub: Arc<tokio::sync::Mutex<Hub>>,
    events: broadcast::Sender<MuxEvent>,
    pane_id: String,
    buffer: Option<String>,
) {
    let mut status = None;
    let mut signal = None;
    let mut dead = false;
    let mut empty_status = 0u8;
    for attempt in 0..8 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let Ok(output) = exec.run(&dead_status_args(&pane_id)).await else {
            continue;
        };
        let Ok(pane) = parse_dead_status(&output.stdout) else {
            continue;
        };
        if !pane.dead {
            continue;
        }
        dead = true;
        status = pane.status;
        signal.clone_from(&pane.signal);
        if status.is_some() || signal.is_some() {
            break;
        }
        empty_status = empty_status.saturating_add(1);
        if empty_status >= 4 {
            break;
        }
    }
    if dead && hub.lock().await.should_emit(&pane_id) {
        drop(events.send(MuxEvent::PaneDied {
            pane_id,
            status,
            signal,
        }));
    }
    if dead {
        delete_buffer(&exec, buffer.as_deref()).await;
    }
}

async fn delete_buffer(exec: &Arc<dyn TmuxExec>, buffer: Option<&str>) {
    if let Some(buffer) = buffer {
        drop(exec.run(&delete_buffer_args(buffer)).await);
    }
}

async fn emit_window_close(ctx: &ControlCtx, window_id: &str) {
    let Some((session, panes)) = ctx.hub.lock().await.take_window(window_id) else {
        return;
    };
    drop(ctx.events.send(MuxEvent::WindowClosed {
        window_id: window_id.to_owned(),
        session,
    }));
    let mut hub = ctx.hub.lock().await;
    for pane_id in panes {
        if hub.should_emit(&pane_id) {
            drop(ctx.events.send(MuxEvent::PaneDied {
                pane_id,
                status: None,
                signal: None,
            }));
        }
    }
}
