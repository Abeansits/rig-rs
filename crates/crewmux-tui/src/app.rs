// Derived from openrig packages/tui/src/main.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
//! The Elm loop. I/O stays here; [`crate::update`] is the only place state changes.

use std::io::{self, Stdout};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind};
use futures_util::StreamExt;
use parking_lot::Mutex;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crewmux_api::{Client, Snapshot};

use crate::attach::{self, LaunchFn};
use crate::control::{self, Incoming};
use crate::error::Error;
use crate::guard::{self, CrosstermGuard, TerminalGuard};
use crate::msg::{Cmd, HydrateFail, KeyEvent, KeyKind, MouseEvent, Msg};
use crate::refresh::{SingleFlight, Ticket};
use crate::sse;
use crate::state::{self, SessionEnv, Size, State};
use crate::update::update;
use crate::view::view;

/// Process configuration. The binary reads the environment and passes it in.
pub struct Config {
    /// Daemon Unix socket. Hydrate and the event stream both use it.
    pub daemon_socket: PathBuf,
    /// Control socket path, already passed through the `sun_path` guard.
    pub control_socket: PathBuf,
    /// Attach policy.
    pub env: SessionEnv,
    /// Herdr or Ghostty opener. Absent when no launcher is configured.
    pub launch: Option<LaunchFn>,
}

/// Run the TUI on a private multi-thread runtime.
///
/// # Errors
///
/// Returns [`Error`] when the runtime or the terminal cannot start.
pub fn run_blocking(config: Config) -> Result<(), Error> {
    adopt_pty();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(Error::Runtime)?;
    runtime.block_on(run(config))
}

/// Run until the operator quits or the terminal fails.
///
/// # Errors
///
/// Returns [`Error`] when stdout is not a terminal or a mode switch fails.
pub async fn run(config: Config) -> Result<(), Error> {
    if !io::IsTerminal::is_terminal(&io::stdout()) {
        return Err(Error::Terminal(io::Error::new(
            io::ErrorKind::Unsupported,
            "stdout is not a terminal",
        )));
    }
    let cancel = CancellationToken::new();
    let tracker = TaskTracker::new();
    let (control_tx, control_rx) = mpsc::channel(8);
    let (reply_tx, reply_rx) = mpsc::channel(8);
    let socket = config.control_socket.clone();
    let socket_cancel = cancel.clone();
    tracker.spawn(async move {
        if let Err(err) = control::serve(&socket, socket_cancel, control_tx, reply_rx).await {
            tracing::error!(error = %err, "control socket ended");
        }
    });
    let (event_tx, event_rx) = mpsc::channel(32);
    let (status_tx, status_rx) = mpsc::channel(8);
    let stream_socket = config.daemon_socket.clone();
    let stream_cancel = cancel.clone();
    sse::spawn(
        &tracker,
        stream_cancel,
        std::time::Duration::from_secs(1),
        move |last_id| {
            let socket = stream_socket.clone();
            async move { sse::open_unix(&socket, last_id).await }
        },
        event_tx,
        status_tx,
    );
    let mut guard = CrosstermGuard;
    guard::install_terminal_hook();
    guard.resume().map_err(Error::Terminal)?;
    let mut terminal =
        Terminal::new(CrosstermBackend::new(io::stdout())).map_err(Error::Terminal)?;
    let mut app = App::new(config, crossterm_size());
    let (ticket_tx, ticket_rx) = mpsc::channel(4);
    let (done_tx, done_rx) = mpsc::channel(4);
    let slot = Arc::clone(&app.slot);
    let watch_cancel = cancel.clone();
    tracker.spawn(async move {
        watch_hydrates(watch_cancel, ticket_rx, done_tx, slot).await;
    });
    let result = drive(
        &mut app,
        &mut terminal,
        &tracker,
        &cancel,
        DriveIo {
            events: event_rx,
            status: status_rx,
            control: control_rx,
            reply_tx,
            ticket_tx,
            done_rx,
        },
    )
    .await;
    cancel.cancel();
    tracker.close();
    tracker.wait().await;
    if let Err(err) = guard.suspend() {
        tracing::debug!(error = %err, "suspend on shutdown");
    }
    result
}

struct DriveIo {
    events: mpsc::Receiver<crewmux_api::StreamEvent>,
    status: mpsc::Receiver<crate::state::ConnState>,
    control: mpsc::Receiver<Incoming>,
    reply_tx: mpsc::Sender<(crate::msg::ControlReplyId, serde_json::Value)>,
    ticket_tx: mpsc::Sender<Ticket>,
    done_rx: mpsc::Receiver<Result<Snapshot, HydrateFail>>,
}

struct Links<'a> {
    tracker: &'a TaskTracker,
    cancel: &'a CancellationToken,
    reply_tx: &'a mpsc::Sender<(crate::msg::ControlReplyId, serde_json::Value)>,
    ticket_tx: &'a mpsc::Sender<Ticket>,
}

struct Hydrator {
    socket: PathBuf,
    slot: Arc<Mutex<Option<Result<Snapshot, HydrateFail>>>>,
}

impl Clone for Hydrator {
    fn clone(&self) -> Self {
        Self {
            socket: self.socket.clone(),
            slot: Arc::clone(&self.slot),
        }
    }
}

impl Hydrator {
    async fn run(&self) -> Result<(), Arc<str>> {
        match fetch(&self.socket).await {
            Ok(snapshot) => {
                *self.slot.lock() = Some(Ok(snapshot));
                Ok(())
            }
            Err(err) => {
                *self.slot.lock() = Some(Err(err));
                Err(Arc::from("hydrate"))
            }
        }
    }
}

struct App {
    state: State,
    daemon_socket: PathBuf,
    launch: Option<LaunchFn>,
    gate: SingleFlight,
    slot: Arc<Mutex<Option<Result<Snapshot, HydrateFail>>>>,
    /// Flight id currently watched, if any.
    watching: Option<u64>,
    /// A newer flight that started before the watcher reported completion.
    queued: Option<Ticket>,
    quit: bool,
}

impl App {
    fn new(config: Config, size: Size) -> Self {
        let slot = Arc::new(Mutex::new(None));
        let daemon_socket = config.daemon_socket.clone();
        let hydrator = Hydrator {
            socket: config.daemon_socket,
            slot: Arc::clone(&slot),
        };
        let gate = SingleFlight::new(move || {
            let hydrator = hydrator.clone();
            async move { hydrator.run().await }
        });
        let mut state = State::new(size, Instant::now());
        state.env = config.env;
        Self {
            state,
            daemon_socket,
            launch: config.launch,
            gate,
            slot,
            watching: None,
            queued: None,
            quit: false,
        }
    }
}

async fn watch_hydrates(
    cancel: CancellationToken,
    mut tickets: mpsc::Receiver<Ticket>,
    done: mpsc::Sender<Result<Snapshot, HydrateFail>>,
    slot: Arc<Mutex<Option<Result<Snapshot, HydrateFail>>>>,
) {
    loop {
        let ticket = tokio::select! {
            () = cancel.cancelled() => return,
            ticket = tickets.recv() => ticket,
        };
        let Some(mut ticket) = ticket else { return };
        let result = ticket.wait().await;
        let body = take_slot(&slot, &result);
        if done.send(body).await.is_err() {
            return;
        }
    }
}

async fn drive(
    app: &mut App,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    tracker: &TaskTracker,
    cancel: &CancellationToken,
    mut io: DriveIo,
) -> Result<(), Error> {
    let mut reader = EventStream::new();
    let links = Links {
        tracker,
        cancel,
        reply_tx: &io.reply_tx,
        ticket_tx: &io.ticket_tx,
    };
    apply(
        app,
        Msg::Key(KeyEvent {
            kind: KeyKind::Char('r'),
        }),
        terminal,
        &links,
    )
    .await?;
    loop {
        paint(terminal, app)?;
        if app.quit {
            return Ok(());
        }
        let wait = state::idle_wait(&app.state);
        tokio::select! {
            biased;
            () = cancel.cancelled() => return Ok(()),
            event = reader.next() => {
                if let Some(Ok(event)) = event {
                    on_crossterm(app, &event, terminal, &links).await?;
                }
            }
            Some(event) = io.events.recv() => {
                apply(app, Msg::Live(event), terminal, &links).await?;
            }
            Some(conn) = io.status.recv() => {
                apply(app, Msg::Conn(conn), terminal, &links).await?;
            }
            Some(incoming) = io.control.recv() => {
                apply(app, Msg::Control(incoming.cmd), terminal, &links).await?;
            }
            Some(body) = io.done_rx.recv() => {
                finish_hydrate(app, body, terminal, &links).await?;
            }
            () = tokio::time::sleep(wait) => {
                apply(app, Msg::Tick, terminal, &links).await?;
            }
            result = tokio::signal::ctrl_c() => {
                if result.is_ok() {
                    quit(app, terminal, &links).await?;
                }
            }
        }
    }
}

async fn finish_hydrate(
    app: &mut App,
    body: Result<Snapshot, crate::msg::HydrateFail>,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    links: &Links<'_>,
) -> Result<(), Error> {
    app.watching = None;
    apply(app, Msg::Hydrated(body), terminal, links).await?;
    flush_queued(app, links.ticket_tx).await;
    Ok(())
}

async fn quit(
    app: &mut App,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    links: &Links<'_>,
) -> Result<(), Error> {
    apply(
        app,
        Msg::Key(KeyEvent {
            kind: KeyKind::Char('q'),
        }),
        terminal,
        links,
    )
    .await
}

async fn flush_queued(app: &mut App, ticket_tx: &mpsc::Sender<Ticket>) {
    let Some(ticket) = app.queued.take() else {
        return;
    };
    app.watching = Some(ticket.id);
    if ticket_tx.send(ticket).await.is_err() {
        tracing::debug!("hydrate watcher stopped");
    }
}

fn take_slot(
    slot: &Mutex<Option<Result<Snapshot, HydrateFail>>>,
    result: &Result<(), Arc<str>>,
) -> Result<Snapshot, HydrateFail> {
    match slot.lock().take() {
        Some(body) => body,
        None if result.is_err() => Err(HydrateFail::Unreachable),
        None => Err(HydrateFail::BadBody),
    }
}

async fn on_crossterm(
    app: &mut App,
    event: &Event,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    links: &Links<'_>,
) -> Result<(), Error> {
    let Some(msg) = decode_event(event) else {
        return Ok(());
    };
    apply(app, msg, terminal, links).await
}

fn decode_event(event: &Event) -> Option<Msg> {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "v1 ignores terminal events it does not bind"
    )]
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c' | 'C'))
            {
                return Some(Msg::Key(KeyEvent {
                    kind: KeyKind::Char('q'),
                }));
            }
            key_msg(key.code)
        }
        Event::Mouse(mouse) => mouse_msg(mouse.kind, mouse.column, mouse.row),
        Event::Resize(cols, rows) => Some(Msg::Resize(*cols, *rows)),
        _ => None,
    }
}

fn key_msg(code: KeyCode) -> Option<Msg> {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "v1 ignores keys it does not bind"
    )]
    let kind = match code {
        KeyCode::Char(ch) => KeyKind::Char(ch),
        KeyCode::Up => KeyKind::Up,
        KeyCode::Down => KeyKind::Down,
        KeyCode::Enter => KeyKind::Enter,
        _ => return None,
    };
    Some(Msg::Key(KeyEvent { kind }))
}

fn mouse_msg(kind: MouseEventKind, column: u16, row: u16) -> Option<Msg> {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "v1 ignores mouse buttons other than primary and the wheel"
    )]
    let button = match kind {
        MouseEventKind::Down(crossterm::event::MouseButton::Left) => 0,
        MouseEventKind::ScrollDown => 65,
        MouseEventKind::ScrollUp => 64,
        _ => return None,
    };
    Some(Msg::Mouse(MouseEvent {
        button,
        x: column.saturating_add(1),
        y: row.saturating_add(1),
    }))
}

async fn apply(
    app: &mut App,
    msg: Msg,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    links: &Links<'_>,
) -> Result<(), Error> {
    let mut pending = vec![msg];
    while let Some(msg) = pending.pop() {
        let placeholder = State::new(app.state.size, app.state.now);
        let state = std::mem::replace(&mut app.state, placeholder);
        let (next, cmds) = update(state, msg);
        app.state = next;
        for cmd in cmds {
            if let Some(follow) = run_cmd(app, cmd, terminal, links).await? {
                pending.push(follow);
            }
        }
    }
    Ok(())
}

async fn run_cmd(
    app: &mut App,
    cmd: Cmd,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    links: &Links<'_>,
) -> Result<Option<Msg>, Error> {
    match cmd {
        Cmd::Hydrate => {
            arm_hydrate(app, links).await;
            Ok(None)
        }
        Cmd::Quit => {
            app.quit = true;
            Ok(None)
        }
        Cmd::Reply(id, value) => {
            if links.reply_tx.send((id, value)).await.is_err() {
                tracing::debug!("control client is gone");
            }
            Ok(None)
        }
        Cmd::Attach(target) => finish_attach(app, target, terminal).await,
    }
}

async fn finish_attach(
    app: &mut App,
    target: crate::msg::AttachTarget,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> Result<Option<Msg>, Error> {
    let mut guard = CrosstermGuard;
    let outcome = attach::perform(target, app.launch.clone(), &mut guard, &app.daemon_socket).await;
    if let Err(err) = terminal.clear() {
        tracing::debug!(error = %err, "clear after attach");
    }
    Ok(Some(Msg::AttachReturned(outcome)))
}

async fn arm_hydrate(app: &mut App, links: &Links<'_>) {
    let ticket = app.gate.request(links.tracker, links.cancel.clone());
    match app.watching {
        Some(id) if id == ticket.id => {}
        Some(_) => app.queued = Some(ticket),
        None => {
            app.watching = Some(ticket.id);
            if links.ticket_tx.send(ticket).await.is_err() {
                tracing::debug!("hydrate watcher stopped");
            }
        }
    }
}

fn paint(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> Result<(), Error> {
    let mut hits = crate::action::HitMap::default();
    terminal
        .draw(|frame| hits = view(&app.state, frame))
        .map_err(Error::Terminal)?;
    let placeholder = State::new(app.state.size, app.state.now);
    let state = std::mem::replace(&mut app.state, placeholder);
    let (next, _) = update(state, Msg::Painted(hits));
    app.state = next;
    Ok(())
}

/// Make this process's controlling terminal the stdout pty when it is not one already.
///
/// Crossterm reads window size from `/dev/tty`. A TUI spawned with stdout on a
/// pty and no controlling terminal would otherwise keep the parent's size, so
/// `SIGWINCH` would not redraw at the pty's columns.
fn adopt_pty() {
    if rustix::process::setsid().is_ok() {
        let _adopted = rustix::process::ioctl_tiocsctty(io::stdout());
    }
}

fn crossterm_size() -> Size {
    match crossterm::terminal::size() {
        Ok((cols, rows)) => Size::new(cols, rows),
        Err(_) => Size::new(80, 24),
    }
}

async fn fetch(path: &std::path::Path) -> Result<Snapshot, HydrateFail> {
    let path = path.to_path_buf();
    match tokio::task::spawn_blocking(move || Client::over_uds(path).hydrate()).await {
        Ok(Ok(snapshot)) => Ok(snapshot),
        Ok(Err(crewmux_api::Error::Connect { .. } | crewmux_api::Error::Exchange { .. })) => {
            Err(HydrateFail::Unreachable)
        }
        Ok(Err(
            crewmux_api::Error::BadPath
            | crewmux_api::Error::BadMethod
            | crewmux_api::Error::BadHeader
            | crewmux_api::Error::Protocol(_)
            | crewmux_api::Error::BodyEncoding
            | crewmux_api::Error::Status { .. }
            | crewmux_api::Error::Json { .. },
        )) => Err(HydrateFail::BadBody),
        Err(_) => Err(HydrateFail::Unreachable),
    }
}
