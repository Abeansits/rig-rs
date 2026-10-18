//! [`CodexAdapter`](crate::CodexAdapter): pre-flight, launch, and readiness.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use async_trait::async_trait;
use crewmux_mux::{CaptureRange, Multiplexer, PaneRef, SessionSpec};
use crewmux_types::{DialogId, ReadinessCode, RuntimeKind, SeatId, ThreadId};
use tokio::sync::{Mutex, OwnedSemaphorePermit};

use crate::adapter::{AgentAdapter, SeatIo};
use crate::codex::argv::{CodexLaunchPlan, codex_launch_argv};
use crate::codex::budget::LaunchBudget;
use crate::codex::dialogs::CODEX_DIALOGS;
use crate::codex::lock::CodexLaunchLock;
use crate::codex::preflight::{Preflight, codex_preflight_bin};
use crate::codex::rollout::{RolloutLocator, RolloutSet, ThreadIdError};
use crate::codex::screen::{CodexScreen, apply_preflight, classify};
use crate::dialog::{AnswerPolicy, DialogDriver, DriveOutcome, MuxPane, SeatContext};
use crate::error::Error;
use crate::types::{
    AppliedLaunch, Capabilities, InstalledResource, LaunchOpts, LaunchOutcome, NodeBinding,
    ProjectionPlan, ProjectionResult, Readiness, ResolvedStartupFile, ResumeToken,
    StartupDeliveryResult,
};

const LOGIN_REMEDY: &str = "Not logged in. Run `codex login` in a terminal.";

/// Launches Codex with a login pre-flight and per-seat `-c` MCP overrides.
pub struct CodexAdapter {
    mux: Arc<dyn Multiplexer>,
    bin: PathBuf,
    budget: LaunchBudget,
    lock: CodexLaunchLock,
    seats: Mutex<HashMap<SeatId, Track>>,
}

struct Track {
    preflight: Preflight,
    home: PathBuf,
    codex_home: PathBuf,
    cwd: PathBuf,
    launched_at: SystemTime,
    deadline: Instant,
    before: RolloutSet,
    permit: Option<OwnedSemaphorePermit>,
    id: IdState,
    policy: AnswerPolicy,
    note: Option<String>,
}

enum IdState {
    Pending,
    Resolved(ThreadId),
    Failed(ReadinessCode),
}

impl CodexAdapter {
    /// `codex` on `PATH`, with [`LaunchBudget::codex`].
    #[must_use]
    pub fn new(mux: Arc<dyn Multiplexer>) -> Self {
        Self::with_bin(mux, PathBuf::from("codex"))
    }

    /// `bin` is used for `login status` and the tmux argv.
    #[must_use]
    pub fn with_bin(mux: Arc<dyn Multiplexer>, bin: PathBuf) -> Self {
        Self {
            mux,
            bin,
            budget: LaunchBudget::codex(),
            lock: CodexLaunchLock::new(),
            seats: Mutex::new(HashMap::new()),
        }
    }

    /// Replace the launch budget. Tests use a short thread-id wait.
    #[must_use]
    pub fn with_budget(mut self, budget: LaunchBudget) -> Self {
        self.budget = budget;
        self
    }

    /// Doctor note when pre-flight output was not recognised. Empty when it was.
    pub async fn preflight_note(&self, seat: SeatId) -> Option<String> {
        self.seats
            .lock()
            .await
            .get(&seat)
            .and_then(|track| track.note.clone())
    }

    /// Resolved rollout id, once one was chosen.
    pub async fn thread_id(&self, seat: SeatId) -> Option<ThreadId> {
        match self.seats.lock().await.get(&seat).map(|track| &track.id) {
            Some(IdState::Resolved(id)) => Some(id.clone()),
            _ => None,
        }
    }
}

#[async_trait]
impl AgentAdapter for CodexAdapter {
    fn runtime(&self) -> RuntimeKind {
        RuntimeKind::Codex
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn launch_timeout(&self) -> std::time::Duration {
        self.budget.overall
    }

    async fn list_installed(&self, _: &NodeBinding) -> Result<Vec<InstalledResource>, Error> {
        Ok(Vec::new())
    }

    async fn project(
        &self,
        _: &ProjectionPlan,
        _: &NodeBinding,
    ) -> Result<ProjectionResult, Error> {
        Ok(ProjectionResult::default())
    }

    async fn deliver_startup(
        &self,
        _: &[ResolvedStartupFile],
        _: &NodeBinding,
        _: &dyn SeatIo,
    ) -> Result<StartupDeliveryResult, Error> {
        Ok(StartupDeliveryResult::default())
    }

    async fn launch(
        &self,
        binding: &NodeBinding,
        opts: LaunchOpts,
    ) -> Result<LaunchOutcome, Error> {
        if !dir_exists(&binding.cwd).await? {
            return Ok(failed_cwd(&binding.cwd));
        }
        let paths = SeatPaths::from_binding(binding, &opts);
        let bin = codex_bin(&self.bin, &opts);
        let preflight = codex_preflight_bin(
            &bin,
            &paths.home,
            &paths.codex_home,
            self.budget().preflight,
        )
        .await;
        let note = unrecognised_note(&preflight);
        if let Some(outcome) = short_circuit(&preflight) {
            self.note_blocked(binding, preflight, &paths, note).await;
            return Ok(outcome);
        }
        let (approval, auto_approve) = policy_from(binding);
        let resume = resume_id(&opts)?;
        if let Some(id) = &resume {
            let home = paths.codex_home.clone();
            let cwd = binding.cwd.clone();
            let id = id.clone();
            blocking(move || verify_existing(&home, &id, &cwd)).await??;
        }
        let (before, permit) = self.prepare_fresh(&paths, resume.is_some()).await?;
        let plan = CodexLaunchPlan {
            bin,
            approval,
            resume: resume.clone(),
            mcp_env: mcp_env(&opts),
            rig_bin: crewmux_provider::rig_executable(),
        };
        let pane = self.start_session(binding, &opts, &paths, &plan).await?;
        self.remember(OpenedSeat {
            binding,
            preflight,
            paths,
            before,
            permit,
            resume,
            auto_approve,
            note,
        })
        .await;
        Ok(LaunchOutcome::Ok {
            resume: None,
            applied: AppliedLaunch {
                tmux_session: pane.session.clone(),
                pane,
            },
        })
    }

    async fn check_ready(&self, binding: &NodeBinding) -> Result<Readiness, Error> {
        if let Some(forced) = self.forced_readiness(binding.seat).await {
            return Ok(forced);
        }
        let text = self.capture(binding).await?;
        let screen = self.screen_for(binding.seat, &text).await;
        let screen = self.answer_known(binding, screen).await?;
        self.refresh_id(binding.seat).await?;
        if let Some(code) = self.id_failure(binding.seat).await {
            return Ok(Readiness::Gate(code));
        }
        Ok(to_readiness(screen))
    }

    async fn submit(
        &self,
        binding: &NodeBinding,
        text: &str,
    ) -> Result<crate::types::DeliveryReceipt, Error> {
        Self::submit(self, binding, text, crate::types::SubmitMode::Enter).await
    }

    async fn discover_resume_token(
        &self,
        binding: &NodeBinding,
    ) -> Result<Option<ResumeToken>, Error> {
        Ok(self.thread_id(binding.seat).await.map(|id| ResumeToken {
            kind: "codex".to_owned(),
            value: id.as_str().to_owned(),
        }))
    }
}

impl CodexAdapter {
    async fn capture(&self, binding: &NodeBinding) -> Result<String, Error> {
        let capture = self
            .mux
            .capture(&binding.pane, CaptureRange::Visible)
            .await?;
        Ok(capture.text)
    }

    async fn screen_for(&self, seat: SeatId, text: &str) -> CodexScreen {
        let screen = classify(text);
        let guard = self.seats.lock().await;
        let Some(track) = guard.get(&seat) else {
            return screen;
        };
        apply_preflight(&track.preflight, screen)
    }

    async fn answer_known(
        &self,
        binding: &NodeBinding,
        screen: CodexScreen,
    ) -> Result<CodexScreen, Error> {
        let CodexScreen::Dialog(id) = screen else {
            return Ok(screen);
        };
        if !matches!(
            id,
            DialogId::CodexUpdate
                | DialogId::CodexMcpToolApproval
                | DialogId::CodexBackgroundServer
        ) {
            return Ok(CodexScreen::Dialog(id));
        }
        let policy = self.policy_for(binding.seat).await;
        let Some((cwd, home)) = self.trust_paths(binding.seat).await else {
            return Ok(CodexScreen::Dialog(id));
        };
        let seat = SeatContext {
            cwd: &cwd,
            home: &home,
        };
        let io = MuxPane::new(Arc::clone(&self.mux), binding.pane.clone());
        if !matches!(
            DialogDriver::live()
                .drive(&io, CODEX_DIALOGS, policy, &seat)
                .await?,
            DriveOutcome::Answered { .. }
        ) {
            return Ok(CodexScreen::Dialog(id));
        }
        Ok(classify(&self.capture(binding).await?))
    }

    async fn policy_for(&self, seat: SeatId) -> AnswerPolicy {
        self.seats.lock().await.get(&seat).map_or(
            AnswerPolicy {
                auto_trust: false,
                auto_approve_rig_tools: false,
            },
            |track| track.policy,
        )
    }

    async fn trust_paths(&self, seat: SeatId) -> Option<(PathBuf, PathBuf)> {
        let guard = self.seats.lock().await;
        let track = guard.get(&seat)?;
        Some((track.cwd.clone(), track.home.clone()))
    }

    async fn prepare_fresh(
        &self,
        paths: &SeatPaths,
        resume: bool,
    ) -> Result<(RolloutSet, Option<OwnedSemaphorePermit>), Error> {
        if resume {
            let locator = locator(&paths.codex_home);
            return Ok((blocking(move || locator.snapshot()).await?, None));
        }
        let permit = self.lock.acquire(&paths.codex_home, &paths.cwd).await;
        let locator = locator(&paths.codex_home);
        let before = blocking(move || locator.snapshot()).await?;
        Ok((before, Some(permit)))
    }

    async fn start_session(
        &self,
        binding: &NodeBinding,
        opts: &LaunchOpts,
        paths: &SeatPaths,
        plan: &CodexLaunchPlan,
    ) -> Result<PaneRef, Error> {
        let mut spec = SessionSpec::new(opts.session_name.clone(), binding.cwd.clone());
        spec.argv = codex_launch_argv(plan);
        spec.env = opts.env.clone();
        spec.env
            .insert("HOME".to_owned(), paths.home.display().to_string());
        spec.env.insert(
            "CODEX_HOME".to_owned(),
            paths.codex_home.display().to_string(),
        );
        spec.remain_on_exit = true;
        Ok(self.mux.create_session(&spec).await?)
    }

    async fn remember(&self, opened: OpenedSeat<'_>) {
        let id = opened.resume.map_or(IdState::Pending, IdState::Resolved);
        let track = Track {
            preflight: opened.preflight,
            home: opened.paths.home,
            codex_home: opened.paths.codex_home,
            cwd: opened.paths.cwd,
            launched_at: SystemTime::now(),
            deadline: Instant::now() + self.budget.overall,
            before: opened.before,
            permit: opened.permit,
            id,
            policy: AnswerPolicy {
                auto_trust: false,
                auto_approve_rig_tools: opened.auto_approve,
            },
            note: opened.note,
        };
        self.seats.lock().await.insert(opened.binding.seat, track);
    }

    async fn note_blocked(
        &self,
        binding: &NodeBinding,
        preflight: Preflight,
        paths: &SeatPaths,
        note: Option<String>,
    ) {
        let track = Track {
            preflight,
            home: paths.home.clone(),
            codex_home: paths.codex_home.clone(),
            cwd: paths.cwd.clone(),
            launched_at: SystemTime::UNIX_EPOCH,
            deadline: Instant::now(),
            before: RolloutSet::default(),
            permit: None,
            id: IdState::Pending,
            policy: AnswerPolicy {
                auto_trust: false,
                auto_approve_rig_tools: false,
            },
            note,
        };
        self.seats.lock().await.insert(binding.seat, track);
    }

    async fn forced_readiness(&self, seat: SeatId) -> Option<Readiness> {
        let guard = self.seats.lock().await;
        let track = guard.get(&seat)?;
        match &track.preflight {
            Preflight::NotLoggedIn => Some(Readiness::Login {
                kind: crewmux_types::LoginKind::PreflightNotLoggedIn,
                evidence: LOGIN_REMEDY.to_owned(),
            }),
            Preflight::CodexHomeMissing => {
                Some(Readiness::HardFailure("codex_home_missing".to_owned()))
            }
            Preflight::Unrecognised { code, out } => {
                Some(Readiness::HardFailure(preflight_failure(*code, out)))
            }
            Preflight::LoggedIn => None,
        }
    }

    pub(crate) fn mux(&self) -> &Arc<dyn Multiplexer> {
        &self.mux
    }

    pub(crate) fn budget(&self) -> LaunchBudget {
        self.budget
    }

    pub(crate) async fn visible_screen(&self, binding: &NodeBinding) -> Result<CodexScreen, Error> {
        if let Some(Readiness::Login { kind, .. }) = self.forced_readiness(binding.seat).await {
            return Ok(CodexScreen::Login(kind));
        }
        let text = self.capture(binding).await?;
        Ok(self.screen_for(binding.seat, &text).await)
    }

    pub(crate) async fn fail_id(&self, seat: SeatId, code: ReadinessCode) {
        let mut guard = self.seats.lock().await;
        if let Some(track) = guard.get_mut(&seat) {
            track.id = IdState::Failed(code);
            track.permit = None;
        }
    }

    pub(crate) async fn store_resolved(&self, seat: SeatId, id: ThreadId) {
        let mut guard = self.seats.lock().await;
        if let Some(track) = guard.get_mut(&seat) {
            track.id = IdState::Resolved(id);
            track.permit = None;
        }
    }

    pub(crate) async fn id_query(&self, seat: SeatId) -> Option<IdQuery> {
        let guard = self.seats.lock().await;
        let track = guard.get(&seat)?;
        Some(IdQuery {
            locator: locator(&track.codex_home),
            cwd: track.cwd.clone(),
            launched_at: track.launched_at,
            before: track.before.clone(),
        })
    }

    async fn refresh_id(&self, seat: SeatId) -> Result<(), Error> {
        let snapshot = {
            let guard = self.seats.lock().await;
            let Some(track) = guard.get(&seat) else {
                return Ok(());
            };
            if !matches!(track.id, IdState::Pending) {
                return Ok(());
            }
            Some((
                track.codex_home.clone(),
                track.cwd.clone(),
                track.launched_at,
                track.before.clone(),
                track.deadline,
            ))
        };
        let Some((home, cwd, launched_at, before, deadline)) = snapshot else {
            return Ok(());
        };
        let locator = locator(&home);
        let resolved = blocking(move || locator.resolve(&before, &cwd, launched_at, None)).await?;
        let mut guard = self.seats.lock().await;
        let Some(track) = guard.get_mut(&seat) else {
            return Ok(());
        };
        match resolved {
            Ok(id) => {
                track.id = IdState::Resolved(id);
                track.permit = None;
            }
            Err(ThreadIdError::NotYet) => {
                if Instant::now() >= deadline {
                    track.permit = None;
                }
            }
            Err(ThreadIdError::Ambiguous(_)) => {
                track.id = IdState::Failed(ReadinessCode::ThreadIdAmbiguous);
                track.permit = None;
            }
            Err(ThreadIdError::Malformed(_)) => {
                track.id = IdState::Failed(ReadinessCode::NotVerified);
                track.permit = None;
            }
        }
        Ok(())
    }

    async fn id_failure(&self, seat: SeatId) -> Option<ReadinessCode> {
        match self.seats.lock().await.get(&seat).map(|track| &track.id) {
            Some(IdState::Failed(code)) => Some(code.clone()),
            _ => None,
        }
    }
}

struct OpenedSeat<'a> {
    binding: &'a NodeBinding,
    preflight: Preflight,
    paths: SeatPaths,
    before: RolloutSet,
    permit: Option<OwnedSemaphorePermit>,
    resume: Option<ThreadId>,
    auto_approve: bool,
    note: Option<String>,
}

/// Filesystem query for the seat's rollout id.
pub(crate) struct IdQuery {
    /// Sessions directory for this seat's `CODEX_HOME`.
    pub locator: RolloutLocator,
    /// Seat working directory.
    pub cwd: PathBuf,
    /// When the tmux session was created.
    pub launched_at: SystemTime,
    /// Rollout files that existed before exec.
    pub before: RolloutSet,
}

struct SeatPaths {
    home: PathBuf,
    codex_home: PathBuf,
    cwd: PathBuf,
}

impl SeatPaths {
    fn from_binding(binding: &NodeBinding, opts: &LaunchOpts) -> Self {
        let home = env_path(opts, "HOME").unwrap_or_else(|| PathBuf::from("."));
        let codex_home = env_path(opts, "CODEX_HOME").unwrap_or_else(|| home.join(".codex"));
        Self {
            home,
            codex_home,
            cwd: binding.cwd.clone(),
        }
    }
}

fn locator(codex_home: &Path) -> RolloutLocator {
    RolloutLocator {
        sessions_root: codex_home.join("sessions"),
    }
}

fn short_circuit(preflight: &Preflight) -> Option<LaunchOutcome> {
    match preflight {
        Preflight::NotLoggedIn => Some(LaunchOutcome::LoginRequired {
            kind: crewmux_types::LoginKind::PreflightNotLoggedIn,
            evidence: LOGIN_REMEDY.to_owned(),
        }),
        Preflight::CodexHomeMissing => Some(LaunchOutcome::Failed {
            error: "codex_home_missing".to_owned(),
            recovery: None,
            evidence: Some("codex home does not exist".to_owned()),
        }),
        Preflight::Unrecognised { code, out } => Some(LaunchOutcome::Failed {
            error: preflight_failure(*code, out),
            recovery: None,
            evidence: Some(out.clone()),
        }),
        Preflight::LoggedIn => None,
    }
}

fn preflight_failure(code: i32, out: &str) -> String {
    if out.starts_with("codex login status") {
        out.to_owned()
    } else {
        format!("codex login status failed: exit {code}: {out}")
    }
}

fn unrecognised_note(preflight: &Preflight) -> Option<String> {
    match preflight {
        Preflight::Unrecognised { code, out } => {
            Some(format!("preflight_unrecognised exit {code}: {out}"))
        }
        Preflight::LoggedIn | Preflight::NotLoggedIn | Preflight::CodexHomeMissing => None,
    }
}

fn policy_from(binding: &NodeBinding) -> (crewmux_types::CodexApproval, bool) {
    let approval = binding
        .runtime_opts
        .get("approval")
        .and_then(serde_json::Value::as_str)
        .map_or(
            crewmux_types::CodexApproval::OnRequest,
            crewmux_types::CodexApproval::parse,
        );
    let auto = binding
        .runtime_opts
        .get("auto_approve_rig_tools")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    (approval, auto)
}

fn resume_id(opts: &LaunchOpts) -> Result<Option<ThreadId>, Error> {
    let Some(token) = &opts.resume else {
        return Ok(None);
    };
    ThreadId::new(&token.value)
        .map(Some)
        .map_err(|err| Error::Message(err.to_string()))
}

fn verify_existing(codex_home: &Path, id: &ThreadId, cwd: &Path) -> Result<(), Error> {
    let locator = locator(codex_home);
    locator
        .verify_resume(id, cwd)
        .map(|_| ())
        .map_err(|err| Error::Message(format!("resume rejected: {err}")))
}

fn mcp_env(opts: &LaunchOpts) -> std::collections::BTreeMap<String, String> {
    let mut env = std::collections::BTreeMap::new();
    for key in [
        crewmux_types::SEAT_ID_ENV,
        crewmux_types::SEAT_TOKEN_ENV,
        "RIG_HOME",
    ] {
        if let Some(value) = opts.env.get(key) {
            env.insert((*key).to_owned(), value.clone());
        }
    }
    env
}

fn codex_bin(default: &Path, opts: &LaunchOpts) -> PathBuf {
    opts.env
        .get("RIG_CODEX_BIN")
        .map_or_else(|| default.to_path_buf(), PathBuf::from)
}

fn env_path(opts: &LaunchOpts, key: &str) -> Option<PathBuf> {
    opts.env
        .get(key)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os(key).map(PathBuf::from))
}

fn failed_cwd(cwd: &Path) -> LaunchOutcome {
    LaunchOutcome::Failed {
        error: format!("cwd does not exist: {}", cwd.display()),
        recovery: Some(crate::types::Recovery::AttentionRequired),
        evidence: Some(cwd.display().to_string()),
    }
}

pub(crate) fn to_readiness(screen: CodexScreen) -> Readiness {
    match screen {
        CodexScreen::Ready(_) => Readiness::Ready,
        CodexScreen::Draft { .. } | CodexScreen::Inconclusive => Readiness::Inconclusive,
        CodexScreen::Busy => Readiness::Booting,
        CodexScreen::Login(kind) => Readiness::Login {
            kind,
            evidence: LOGIN_REMEDY.to_owned(),
        },
        CodexScreen::StaleServer => Readiness::Gate(ReadinessCode::StaleCodexServer),
        CodexScreen::UnknownMenu { .. } => Readiness::Gate(ReadinessCode::UnknownMenu),
        CodexScreen::ReturnedToShell => Readiness::HardFailure("returned_to_shell".to_owned()),
        CodexScreen::HardFailure { reason } => Readiness::HardFailure(reason),
        CodexScreen::Dialog(id) => Readiness::Gate(dialog_code(id)),
    }
}

fn dialog_code(id: DialogId) -> ReadinessCode {
    match id {
        DialogId::CodexUpdate => ReadinessCode::UpdateGate,
        DialogId::CodexMcpToolApproval => ReadinessCode::McpToolApproval,
        DialogId::CodexBackgroundServer => ReadinessCode::CodexBackgroundServer,
        DialogId::ClaudeTrust => ReadinessCode::TrustGate,
        DialogId::ClaudeBypassConsent => ReadinessCode::BypassConsentGate,
        DialogId::ClaudeNewMcpServer => ReadinessCode::McpGate,
        DialogId::ClaudeFullscreenRenderer => ReadinessCode::FullscreenRenderer,
    }
}

async fn dir_exists(path: &Path) -> Result<bool, Error> {
    let path = path.to_path_buf();
    blocking(move || path.is_dir()).await
}

/// Run `job` off the tokio worker. Filesystem and process waits belong here.
pub(crate) async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> T + Send + 'static,
) -> Result<T, Error> {
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|err| Error::Message(err.to_string()))
}
