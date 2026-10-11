//! In-memory [`Multiplexer`](crate::Multiplexer) for tests that must not start tmux.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use tokio::sync::{Mutex, broadcast, watch};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::Key;
use crate::Multiplexer;
use crate::argv::{
    armed_session_args, canonical_session_name, invocation, link_into_control_args,
    manual_size_args, pane_target, reject_global_window_size, resize_args, send_keys_args,
    set_option_args,
};
use crate::clients::{filter_human_clients, last_human_keystroke};
use crate::error::MuxError;
use crate::exec::{ExecOutput, TmuxExec};
use crate::hub::Hub;
use crate::paste::perform_paste;
use crate::types::{
    Capture, CaptureRange, ClientInfo, DeathStream, GuardInputs, MuxEvent, OptScope, OutputStream,
    PaneInfo, PaneRef, PasteOpts, PasteReceipt, SessionSpec,
};

struct FakeInner {
    socket: String,
    sessions: HashMap<String, PaneInfo>,
    calls: Vec<Vec<String>>,
    clients: Vec<ClientInfo>,
    capture: Capture,
    screens: HashMap<String, Capture>,
    script_stubs: bool,
    in_mode: bool,
    scripted: Option<GuardInputs>,
    fail_paste: bool,
    next_id: u32,
    hub: Hub,
}

/// Scripted multiplexer. Records paste and create argv and can inject events.
pub struct FakeMux {
    inner: Arc<Mutex<FakeInner>>,
    events: broadcast::Sender<MuxEvent>,
    own_write: watch::Sender<Option<Instant>>,
    cancel: CancellationToken,
    tracker: TaskTracker,
}

impl Default for FakeMux {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeMux {
    /// A fake bound to the default `rig-rs` socket name.
    #[must_use]
    pub fn new() -> Self {
        Self::with_socket("rig-rs")
    }

    /// A fake that records `-L socket` on every argv.
    #[must_use]
    pub fn with_socket(socket: impl Into<String>) -> Self {
        let (events, _) = broadcast::channel(64);
        let (own_write, _) = watch::channel(None);
        Self {
            inner: Arc::new(Mutex::new(FakeInner {
                socket: socket.into(),
                sessions: HashMap::new(),
                calls: Vec::new(),
                clients: Vec::new(),
                capture: Capture::new(String::new(), (0, 0)),
                screens: HashMap::new(),
                script_stubs: false,
                in_mode: false,
                scripted: None,
                fail_paste: false,
                next_id: 1,
                hub: Hub::default(),
            })),
            events,
            own_write,
            cancel: CancellationToken::new(),
            tracker: TaskTracker::new(),
        }
    }

    /// Honour `RIG_STUB_SCRIPT` on later [`create_session`](Multiplexer::create_session) calls.
    pub async fn enable_stub_scripts(&self) {
        self.inner.lock().await.script_stubs = true;
    }

    fn arm_stub(&self, session: &str, pane_id: &str, plan: &crate::fake_script::StubPlan) {
        if plan.ready_after_ms > 0 {
            let inner = Arc::clone(&self.inner);
            let token = self.cancel.clone();
            let session = session.to_owned();
            let delay = plan.ready_after_ms;
            let text = crate::fake_script::render_stub(plan);
            self.tracker.spawn(async move {
                tokio::select! {
                    () = token.cancelled() => {}
                    () = tokio::time::sleep(Duration::from_millis(delay)) => {
                        let mut guard = inner.lock().await;
                        guard.screens.insert(session, Capture::new(text, (0, 0)));
                    }
                }
            });
        }
        let Some(delay) = plan.exit_after_ms else {
            return;
        };
        let inner = Arc::clone(&self.inner);
        let events = self.events.clone();
        let token = self.cancel.clone();
        let session = session.to_owned();
        let pane_id = pane_id.to_owned();
        self.tracker.spawn(async move {
            tokio::select! {
                () = token.cancelled() => {}
                () = tokio::time::sleep(Duration::from_millis(delay)) => {
                    let mut guard = inner.lock().await;
                    if let Some(info) = guard.sessions.get_mut(&session) {
                        info.dead = true;
                        info.dead_status = Some(0);
                    }
                    drop(guard);
                    drop(events.send(MuxEvent::PaneDied {
                        pane_id,
                        status: Some(0),
                    }));
                }
            }
        });
    }

    /// Argv recorded so far, including `tmux -L <socket> -f /dev/null`.
    pub async fn recorded(&self) -> Vec<Vec<String>> {
        self.inner.lock().await.calls.clone()
    }

    /// Next `paste` fails after `load-buffer` and must `delete-buffer`.
    pub async fn fail_next_paste(&self) {
        self.inner.lock().await.fail_paste = true;
    }

    /// Replace the client list. `control_mode` rows are filtered by [`human_clients`](Multiplexer::human_clients).
    pub async fn set_clients(&self, clients: Vec<ClientInfo>) {
        self.inner.lock().await.clients = clients;
    }

    /// Screen returned by [`capture`](Multiplexer::capture) and unscripted guard tails.
    pub async fn set_capture(&self, capture: Capture) {
        self.inner.lock().await.capture = capture;
    }

    /// Value returned by [`pane_in_mode`](Multiplexer::pane_in_mode) when no guard is scripted.
    pub async fn set_in_mode(&self, in_mode: bool) {
        self.inner.lock().await.in_mode = in_mode;
    }

    /// [`guard_inputs`](Multiplexer::guard_inputs) returns this value unchanged.
    pub async fn script_guard(&self, inputs: GuardInputs) {
        self.inner.lock().await.scripted = Some(inputs);
    }

    /// Push an event onto [`events`](Multiplexer::events). Output also reaches subscribers.
    pub async fn inject(&self, event: MuxEvent) {
        if let MuxEvent::Output {
            ref pane_id,
            ref bytes,
        } = event
        {
            self.inner
                .lock()
                .await
                .hub
                .push_chunk(pane_id, bytes, &self.events);
            return;
        }
        drop(self.events.send(event));
    }
}

impl Drop for FakeMux {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.tracker.close();
    }
}

struct RecordingExec {
    inner: Arc<Mutex<FakeInner>>,
}

#[async_trait]
impl TmuxExec for RecordingExec {
    async fn run(&self, args: &[String]) -> Result<ExecOutput, MuxError> {
        let mut guard = self.inner.lock().await;
        let fail = guard.fail_paste && args.first().map(String::as_str) == Some("paste-buffer");
        let recorded = invocation("tmux", &guard.socket, args);
        guard.calls.push(recorded);
        drop(guard);
        if fail {
            return Err(MuxError::Exec {
                argv: args.to_vec(),
                code: Some(1),
                stderr: "can't find pane".to_owned(),
            });
        }
        Ok(ExecOutput {
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}

fn record(inner: &mut FakeInner, args: &[String]) {
    let recorded = invocation("tmux", &inner.socket, args);
    inner.calls.push(recorded);
}

fn seat(spec: &SessionSpec, session: &str, pane_id: &str) -> PaneInfo {
    let current_cmd = spec
        .argv
        .last()
        .cloned()
        .unwrap_or_else(|| "shell".to_owned());
    PaneInfo {
        pane_id: pane_id.to_owned(),
        window_id: format!("@{pane_id}"),
        session: session.to_owned(),
        pid: Some(1),
        current_cmd,
        dead: false,
        dead_status: None,
        size: spec.geometry,
        in_mode: false,
    }
}

#[async_trait]
impl Multiplexer for FakeMux {
    async fn create_session(&self, spec: &SessionSpec) -> Result<PaneRef, MuxError> {
        let mut guard = self.inner.lock().await;
        let session = canonical_session_name(&spec.name);
        let pane_id = format!("%{}", guard.next_id);
        guard.next_id = guard.next_id.saturating_add(1);
        let socket = guard.socket.clone();
        let armed = armed_session_args(spec, "tmux", &socket, "@bell")?;
        record(&mut guard, &armed);
        record(&mut guard, &manual_size_args(&session));
        record(
            &mut guard,
            &resize_args(&session, spec.geometry.cols, spec.geometry.rows),
        );
        record(&mut guard, &link_into_control_args(&session));
        let info = seat(spec, &session, &pane_id);
        guard.sessions.insert(session.clone(), info);
        let panes: Vec<PaneInfo> = guard.sessions.values().cloned().collect();
        guard.hub.note_panes(&panes);
        let script = scripted_plan(&guard, spec);
        if let Some(plan) = &script {
            let text = if plan.ready_after_ms == 0 {
                crate::fake_script::render_stub(plan)
            } else {
                crate::fake_script::banner().to_owned()
            };
            guard
                .screens
                .insert(session.clone(), Capture::new(text, (0, 0)));
            guard.clients.push(ClientInfo {
                tty: String::new(),
                session: session.clone(),
                activity: SystemTime::UNIX_EPOCH,
                control_mode: true,
            });
        }
        drop(guard);
        if let Some(plan) = script {
            self.arm_stub(&session, &pane_id, &plan);
        }
        Ok(PaneRef {
            session,
            window: Some(0),
            pane: Some(0),
            pane_id: Some(pane_id),
        })
    }

    async fn has_session(&self, name: &str) -> Result<bool, MuxError> {
        let guard = self.inner.lock().await;
        Ok(guard.sessions.contains_key(&canonical_session_name(name)))
    }

    async fn kill_session(&self, name: &str) -> Result<(), MuxError> {
        let mut guard = self.inner.lock().await;
        let name = canonical_session_name(name);
        if guard.sessions.remove(&name).is_none() {
            return Err(MuxError::NotFound(name));
        }
        guard.screens.remove(&name);
        Ok(())
    }

    async fn list_panes(&self) -> Result<Vec<PaneInfo>, MuxError> {
        let guard = self.inner.lock().await;
        Ok(guard.sessions.values().cloned().collect())
    }

    async fn send_keys(&self, pane: &PaneRef, keys: &[Key]) -> Result<(), MuxError> {
        let batches = send_keys_args(&pane_target(pane), keys)?;
        let mut guard = self.inner.lock().await;
        for batch in &batches {
            record(&mut guard, batch);
        }
        drop(guard);
        self.own_write.send_replace(Some(Instant::now()));
        Ok(())
    }

    async fn paste(
        &self,
        pane: &PaneRef,
        text: &str,
        opts: PasteOpts,
    ) -> Result<PasteReceipt, MuxError> {
        let exec = RecordingExec {
            inner: Arc::clone(&self.inner),
        };
        let receipt = perform_paste(&exec, &pane_target(pane), text, opts).await?;
        self.inner.lock().await.fail_paste = false;
        self.own_write.send_replace(Some(Instant::now()));
        Ok(receipt)
    }

    async fn capture(&self, pane: &PaneRef, range: CaptureRange) -> Result<Capture, MuxError> {
        let guard = self.inner.lock().await;
        let session = canonical_session_name(&pane.session);
        let capture = guard
            .screens
            .get(&session)
            .cloned()
            .unwrap_or_else(|| guard.capture.clone());
        Ok(slice_capture(capture, range))
    }

    async fn subscribe_output(&self, pane: &PaneRef) -> Result<OutputStream, MuxError> {
        let pane_id = pane
            .pane_id
            .clone()
            .ok_or_else(|| MuxError::Control("pane id is required".into()))?;
        Ok(self.inner.lock().await.hub.subscribe(&pane_id))
    }

    async fn set_option(&self, scope: OptScope, key: &str, value: &str) -> Result<(), MuxError> {
        reject_global_window_size(&scope, key)?;
        let mut guard = self.inner.lock().await;
        record(&mut guard, &set_option_args(&scope, key, value));
        Ok(())
    }

    async fn human_clients(&self, session: &str) -> Result<Vec<ClientInfo>, MuxError> {
        let guard = self.inner.lock().await;
        Ok(filter_human_clients(
            &guard.clients,
            &canonical_session_name(session),
        ))
    }

    async fn pane_in_mode(&self, _pane: &PaneRef) -> Result<bool, MuxError> {
        Ok(self.inner.lock().await.in_mode)
    }

    async fn guard_inputs(&self, pane: &PaneRef, tail_lines: u16) -> Result<GuardInputs, MuxError> {
        if let Some(scripted) = self.inner.lock().await.scripted.clone() {
            return Ok(scripted);
        }
        let human_clients = self.human_clients(&pane.session).await?;
        let last_human_keystroke_tmux = last_human_keystroke(&human_clients, &pane.session);
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
}

fn scripted_plan(guard: &FakeInner, spec: &SessionSpec) -> Option<crate::fake_script::StubPlan> {
    if !guard.script_stubs {
        return None;
    }
    let raw = spec.env.get("RIG_STUB_SCRIPT").map_or("", String::as_str);
    Some(crate::fake_script::parse_stub(raw))
}

fn slice_capture(capture: Capture, range: CaptureRange) -> Capture {
    let lines = match range {
        CaptureRange::Visible => return capture,
        CaptureRange::LastLines(n) => usize::from(n),
        CaptureRange::History(n) => usize::try_from(n).unwrap_or(usize::MAX),
    };
    let kept: Vec<&str> = capture.text.lines().collect();
    let skip = kept.len().saturating_sub(lines);
    let text = kept.into_iter().skip(skip).collect::<Vec<_>>().join("\n");
    Capture::new(text, capture.cursor)
}
