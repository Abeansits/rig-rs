//! In-memory [`Multiplexer`](crate::Multiplexer) for tests that must not start tmux.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use bytes::Bytes;
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
    /// Frames returned by `capture`, advanced once per `send_keys` call.
    replay: VecDeque<Capture>,
    screens: HashMap<String, Capture>,
    script_stubs: bool,
    in_mode: bool,
    scripted: Option<GuardInputs>,
    fail_paste: bool,
    after_paste: Option<Capture>,
    next_id: u32,
    guard_reads: u32,
    hooks: ScriptHooks,
    hub: Hub,
}

struct ScriptHooks {
    keystroke_on_read: Option<u32>,
    contaminate: bool,
    swallow: bool,
}

/// Scripted multiplexer. Records paste and create argv and can inject events.
pub struct FakeMux {
    inner: Arc<Mutex<FakeInner>>,
    events: broadcast::Sender<MuxEvent>,
    own_write: watch::Sender<Option<Instant>>,
    cancel: CancellationToken,
    tracker: TaskTracker,
    /// `-L` name, used only to build the fake `-S` path.
    label: String,
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
        let socket = socket.into();
        let (events, _) = broadcast::channel(64);
        let (own_write, _) = watch::channel(None);
        Self {
            label: socket.clone(),
            inner: Arc::new(Mutex::new(FakeInner {
                socket,
                sessions: HashMap::new(),
                calls: Vec::new(),
                clients: Vec::new(),
                capture: Capture::new(String::new(), (0, 0)),
                replay: VecDeque::new(),
                screens: HashMap::new(),
                script_stubs: false,
                in_mode: false,
                scripted: None,
                fail_paste: false,
                after_paste: None,
                next_id: 1,
                guard_reads: 0,
                hooks: ScriptHooks {
                    keystroke_on_read: None,
                    contaminate: false,
                    swallow: false,
                },
                hub: Hub::default(),
            })),
            events,
            own_write,
            cancel: CancellationToken::new(),
            tracker: TaskTracker::new(),
        }
    }

    /// On the `n`th [`guard_inputs`](Multiplexer::guard_inputs) call, record a fresh human keystroke.
    pub async fn keystroke_on_guard_read(&self, n: u32) {
        self.inner.lock().await.hooks.keystroke_on_read = Some(n);
    }

    /// After a paste, append extra text so post-paste verification sees contamination.
    pub async fn contaminate_on_paste(&self, enabled: bool) {
        self.inner.lock().await.hooks.contaminate = enabled;
    }

    /// Replace the pane process the next delivery check will see.
    pub async fn set_pane_process(
        &self,
        session: &str,
        pid: Option<u32>,
        command: &str,
        argv: Option<&str>,
    ) {
        let mut guard = self.inner.lock().await;
        let name = canonical_session_name(session);
        let Some(info) = guard.sessions.get_mut(&name) else {
            return;
        };
        info.pid = pid;
        command.clone_into(&mut info.current_cmd);
        if let Some(argv) = argv {
            info.argv = Some(argv.to_owned());
        }
    }

    /// Record the paste argv but leave the screen unchanged, so the payload is missing.
    pub async fn swallow_paste(&self, enabled: bool) {
        self.inner.lock().await.hooks.swallow = enabled;
    }

    /// Honour `RIG_STUB_SCRIPT` on later [`create_session`](Multiplexer::create_session) calls.
    pub async fn enable_stub_scripts(&self) {
        self.inner.lock().await.script_stubs = true;
    }

    /// Put a ready `stub>` prompt back. Repeated pastes otherwise see the last payload.
    pub async fn show_stub_prompt(&self, session: &str) {
        let session = canonical_session_name(session);
        let screen = Capture::new("stub-agent\nstub> \n".to_owned(), (0, 0));
        let mut guard = self.inner.lock().await;
        guard.screens.insert(session, screen);
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

    /// The next paste replaces the screen with `capture` instead of splicing.
    pub async fn set_capture_after_paste(&self, capture: Capture) {
        self.inner.lock().await.after_paste = Some(capture);
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
    ///
    /// Clears any [`replay_screens`](Self::replay_screens) queue.
    pub async fn set_capture(&self, capture: Capture) {
        let mut guard = self.inner.lock().await;
        guard.capture = capture;
        guard.replay.clear();
    }

    /// Screens for a dialog replay. `capture` returns the front. Each `send_keys`
    /// call drops the front once another frame is queued, so a test can show the
    /// cursor moving before Enter.
    pub async fn replay_screens(&self, frames: Vec<Capture>) {
        self.inner.lock().await.replay = VecDeque::from(frames);
    }

    /// Value returned by [`pane_in_mode`](Multiplexer::pane_in_mode) when no guard is scripted.
    pub async fn set_in_mode(&self, in_mode: bool) {
        self.inner.lock().await.in_mode = in_mode;
    }

    /// [`guard_inputs`](Multiplexer::guard_inputs) returns this value unchanged.
    pub async fn script_guard(&self, inputs: GuardInputs) {
        self.inner.lock().await.scripted = Some(inputs);
    }

    /// `%output` bytes for `pane_id`, as the transcript writer consumes them.
    pub async fn inject_output(&self, pane_id: &str, text: &str) {
        self.inject(MuxEvent::Output {
            pane_id: pane_id.to_owned(),
            bytes: Bytes::from(text.to_owned()),
        })
        .await;
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
        // Stable token. Tests must not read the real start time of pid 1.
        started_ticks: Some(7),
        current_cmd,
        argv: Some(spec.argv.join(" ")),
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
        if keys
            .iter()
            .any(|key| matches!(key, Key::CtrlU | Key::CtrlG))
        {
            clear_stub_draft(&mut guard, pane);
        }
        if keys.iter().any(|key| matches!(key, Key::CtrlU)) && guard.replay.is_empty() {
            clear_composer(&mut guard, pane);
        }
        if guard.script_stubs && keys.iter().any(|key| matches!(key, Key::Enter)) {
            let session = canonical_session_name(&pane.session);
            if let Some(screen) = guard.screens.get(&session).cloned() {
                let mut text = screen.text;
                if !text.ends_with("stub> \n") {
                    if !text.ends_with('\n') {
                        text.push('\n');
                    }
                    text.push_str("stub> \n");
                }
                let next = Capture::new(text, (0, 0));
                guard.capture = next.clone();
                guard.screens.insert(session, next);
            }
        }
        if guard.replay.len() > 1 {
            guard.replay.pop_front();
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
        {
            let mut guard = self.inner.lock().await;
            guard.fail_paste = false;
            if let Some(next) = guard.after_paste.take() {
                let session = canonical_session_name(&pane.session);
                if guard.screens.contains_key(&session) {
                    guard.screens.insert(session, next.clone());
                }
                guard.capture = next;
            } else if guard.replay.is_empty() {
                place_pasted_text(&mut guard, pane, text);
            }
        }
        self.own_write.send_replace(Some(Instant::now()));
        if let Some(id) = pane.pane_id.as_deref() {
            if text.ends_with('\n') {
                self.inject_output(id, text).await;
            } else {
                let mut owned = text.to_owned();
                owned.push('\n');
                self.inject_output(id, &owned).await;
            }
        }
        Ok(receipt)
    }

    async fn capture(&self, pane: &PaneRef, range: CaptureRange) -> Result<Capture, MuxError> {
        let guard = self.inner.lock().await;
        let session = canonical_session_name(&pane.session);
        let capture = guard
            .replay
            .front()
            .cloned()
            .or_else(|| guard.screens.get(&session).cloned())
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
        let inject = {
            let mut guard = self.inner.lock().await;
            guard.guard_reads = guard.guard_reads.saturating_add(1);
            let inject = guard.hooks.keystroke_on_read == Some(guard.guard_reads);
            if let Some(scripted) = guard.scripted.clone() {
                return Ok(scripted);
            }
            inject
        };
        if inject {
            let mut guard = self.inner.lock().await;
            guard.clients.push(ClientInfo {
                tty: "tap".to_owned(),
                session: canonical_session_name(&pane.session),
                activity: SystemTime::now(),
                control_mode: false,
            });
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

    fn attach_program(&self) -> (String, String) {
        ("tmux".to_owned(), format!("/tmp/rig-fake-{}", self.label))
    }
}

/// `C-u` restores an empty Claude composer on the capture and the session screen.
fn clear_composer(guard: &mut FakeInner, pane: &PaneRef) {
    guard.capture = clear_prompt(&guard.capture);
    let session = canonical_session_name(&pane.session);
    if let Some(screen) = guard.screens.get_mut(&session) {
        *screen = clear_prompt(screen);
    }
}

/// Show a paste on the screen a later capture will read.
///
/// An empty Claude composer absorbs the payload in place. A stub prompt, which
/// has no such line, keeps the raw text so paste verification can see it.
/// Swallow leaves the screen unchanged. Contamination appends a marker either way.
fn place_pasted_text(guard: &mut FakeInner, pane: &PaneRef, text: &str) {
    let session = canonical_session_name(&pane.session);
    let mut screen = guard
        .screens
        .get(&session)
        .cloned()
        .unwrap_or_else(|| guard.capture.clone());
    if !guard.hooks.swallow {
        screen = land_or_append(screen, text);
    }
    if guard.hooks.contaminate {
        screen.text.push_str(" EXTRA");
        screen = Capture::new(screen.text, screen.cursor);
    }
    let had_screen = guard.screens.contains_key(&session);
    guard.capture = screen.clone();
    if had_screen {
        guard.screens.insert(session, screen);
    }
}

fn land_or_append(screen: Capture, text: &str) -> Capture {
    if screen.text.contains('›') {
        let spliced = spliced_composer(&screen.text, text);
        if spliced != screen.text {
            return Capture::new(spliced, screen.cursor);
        }
    }
    let landed = land_paste(&screen, text);
    if landed.text != screen.text {
        return landed;
    }
    let mut next = screen.text;
    next.push_str(text);
    Capture::new(next, screen.cursor)
}

/// `C-u` drops whatever the paste left after the stub prompt.
fn clear_stub_draft(guard: &mut FakeInner, pane: &PaneRef) {
    let session = canonical_session_name(&pane.session);
    let Some(screen) = guard.screens.get_mut(&session) else {
        return;
    };
    let Some(idx) = screen.text.rfind("stub> ") else {
        return;
    };
    let end = idx.saturating_add("stub> ".len());
    let Some(kept) = screen.text.get(..end) else {
        return;
    };
    let mut text = kept.to_owned();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    let cursor = screen.cursor;
    *screen = Capture::new(text, cursor);
}

fn scripted_plan(guard: &FakeInner, spec: &SessionSpec) -> Option<crate::fake_script::StubPlan> {
    if !guard.script_stubs {
        return None;
    }
    let raw = spec.env.get("RIG_STUB_SCRIPT").map_or("", String::as_str);
    Some(crate::fake_script::parse_stub(raw))
}

/// Put `payload` into an empty `>` / `❯` prompt so a paste check can see it.
///
/// A replay queue is left alone: the test is showing a screen that did not change.
fn land_paste(capture: &Capture, payload: &str) -> Capture {
    let flat = payload.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut lines: Vec<String> = capture.text.lines().map(str::to_owned).collect();
    if let Some(index) = lines.iter().rposition(|line| empty_prompt_line(line))
        && let Some(line) = lines.get_mut(index)
    {
        *line = fill_prompt(line, &flat);
    }
    let mut text = lines.join("\n");
    if capture.text.ends_with('\n') {
        text.push('\n');
    }
    Capture::new(text, capture.cursor)
}

fn empty_prompt_line(line: &str) -> bool {
    let inner = bar_inner(line).unwrap_or(line.trim());
    let Some(body) = strip_prompt_marker(inner) else {
        return false;
    };
    let body = body.trim().trim_matches('\u{a0}');
    body.is_empty() || is_idle_hint(body)
}

fn is_idle_hint(body: &str) -> bool {
    let Some(rest) = body.strip_prefix("Try \"") else {
        return false;
    };
    rest.ends_with('"') && !rest.chars().rev().skip(1).any(|ch| ch == '"')
}

fn strip_prompt_marker(text: &str) -> Option<&str> {
    text.trim()
        .strip_prefix('❯')
        .or_else(|| text.trim().strip_prefix('>'))
        .or_else(|| text.trim().strip_prefix('›'))
}

fn clear_prompt(capture: &Capture) -> Capture {
    let mut lines: Vec<String> = capture.text.lines().map(str::to_owned).collect();
    if let Some(index) = lines.iter().rposition(|line| {
        empty_prompt_line(line) || strip_prompt_marker(line).is_some() || line.contains('│')
    }) && let Some(line) = lines.get_mut(index)
        && (line.contains('❯') || line.contains('>') || line.contains('›'))
    {
        *line = fill_prompt(line, "");
    }
    let mut text = lines.join("\n");
    if capture.text.ends_with('\n') {
        text.push('\n');
    }
    Capture::new(text, capture.cursor)
}

fn fill_prompt(line: &str, flat: &str) -> String {
    let Some(start) = line.find('│') else {
        return rewrite_marker(line, flat);
    };
    let Some(end) = line.rfind('│') else {
        return format!("> {flat}");
    };
    if end <= start {
        return format!("> {flat}");
    }
    let pre = line.get(..start).unwrap_or("");
    let post = line.get(end..).unwrap_or("");
    let inside = line.get(start.saturating_add(1)..end).unwrap_or("");
    let marker = if inside.contains('❯') {
        "❯"
    } else if inside.contains('›') {
        "›"
    } else {
        ">"
    };
    if flat.is_empty() {
        format!("{pre}│ {marker} {post}")
    } else {
        format!("{pre}│ {marker} {flat} {post}")
    }
}

/// Insert `pasted` after the last `›`, keeping a closing box bar on the last line.
fn spliced_composer(capture: &str, pasted: &str) -> String {
    let Some(cursor) = capture.rfind('›') else {
        return capture.to_owned();
    };
    let Some(mark) = capture.get(cursor..).and_then(|rest| rest.chars().next()) else {
        return capture.to_owned();
    };
    let prefix_end = cursor.saturating_add(mark.len_utf8());
    let line_start = capture
        .get(..cursor)
        .and_then(|head| head.rfind('\n'))
        .map_or(0, |index| index.saturating_add(1));
    let line_end = capture
        .get(cursor..)
        .and_then(|rest| rest.find('\n'))
        .map_or(capture.len(), |index| cursor.saturating_add(index));
    let Some(prefix) = capture.get(line_start..prefix_end) else {
        return capture.to_owned();
    };
    let trailing = capture
        .get(prefix_end..line_end)
        .is_some_and(|tail| tail.trim_end().ends_with(['│', '┃']));
    let suffix = if trailing { " │" } else { "" };
    let mut rebuilt = String::new();
    rebuilt.push_str(capture.get(..line_start).unwrap_or(""));
    rebuilt.push_str(prefix);
    rebuilt.push(' ');
    rebuilt.push_str(pasted);
    rebuilt.push_str(suffix);
    rebuilt.push_str(capture.get(line_end..).unwrap_or(""));
    rebuilt
}

fn rewrite_marker(line: &str, flat: &str) -> String {
    let trimmed = line.trim_start();
    let indent_len = line.len().saturating_sub(trimmed.len());
    let indent = line.get(..indent_len).unwrap_or("");
    let marker = if trimmed.starts_with('❯') {
        "❯"
    } else if trimmed.starts_with('›') {
        "›"
    } else {
        ">"
    };
    if flat.is_empty() {
        format!("{indent}{marker}")
    } else {
        format!("{indent}{marker} {flat}")
    }
}

fn bar_inner(line: &str) -> Option<&str> {
    let start = line.find('│')?;
    let end = line.rfind('│')?;
    if end <= start {
        return None;
    }
    line.get(start.saturating_add('│'.len_utf8())..end)
        .map(str::trim)
}

fn slice_capture(capture: Capture, range: CaptureRange) -> Capture {
    let lines = match range {
        CaptureRange::Visible | CaptureRange::Joined => return capture,
        CaptureRange::LastLines(n) => usize::from(n),
        CaptureRange::History(n) | CaptureRange::JoinedHistory(n) => {
            usize::try_from(n).unwrap_or(usize::MAX)
        }
    };
    let trailing = capture.text.ends_with('\n');
    let kept: Vec<&str> = capture.text.lines().collect();
    let count = kept.len();
    let skip = count.saturating_sub(lines);
    let mut text = kept.into_iter().skip(skip).collect::<Vec<_>>().join("\n");
    // `lines` drops the terminator. An exact append is `before + payload`, so a
    // prompt that ended with a newline has to keep it or every paste looks partial.
    if trailing && skip < count {
        text.push('\n');
    }
    Capture::new(text, capture.cursor)
}
