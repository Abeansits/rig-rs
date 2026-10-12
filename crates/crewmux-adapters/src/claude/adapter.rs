//! Claude Code [`crate::AgentAdapter`].

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use crewmux_mux::{CaptureRange, Multiplexer};
use crewmux_types::{ReadinessCode, RuntimeKind, SeatId};
use tokio::sync::Mutex;

use crate::adapter::{AgentAdapter, SeatIo};
use crate::budget::LaunchBudget;
use crate::claude::drive::drive;
use crate::claude::launch::{self, token};
use crate::claude::screen::{classify, readiness};
use crate::claude::submit::ClaudeIo;
use crate::delivery::DeliveryCheck;
use crate::error::Error;
use crate::types::{
    AppliedLaunch, Capabilities, InstalledResource, LaunchOpts, LaunchOutcome, NodeBinding,
    ProjectionPlan, ProjectionResult, Readiness, ResolvedStartupFile, ResumeToken,
    StartupDeliveryResult,
};

/// Launches Claude with a minted session id and per-seat config flags.
pub struct ClaudeAdapter {
    mux: Arc<dyn Multiplexer>,
    check: Arc<dyn DeliveryCheck>,
    seats_root: PathBuf,
    home: PathBuf,
    submit_wait: Duration,
    forced: Mutex<HashMap<SeatId, ReadinessCode>>,
}

impl ClaudeAdapter {
    /// `seats_root` is `$RIG_HOME/seats`. `home` is the directory that holds `.claude`.
    #[must_use]
    pub fn new(
        mux: Arc<dyn Multiplexer>,
        check: Arc<dyn DeliveryCheck>,
        seats_root: PathBuf,
        home: PathBuf,
    ) -> Self {
        Self {
            mux,
            check,
            seats_root,
            home,
            submit_wait: LaunchBudget::submit_wait(),
            forced: Mutex::new(HashMap::new()),
        }
    }

    /// Shorten the transcript wait. Tests use this so a missing turn fails quickly.
    #[must_use]
    pub fn with_submit_wait(mut self, wait: Duration) -> Self {
        self.submit_wait = wait;
        self
    }

    /// Seat directory `$RIG_HOME/seats/<id>`.
    #[must_use]
    pub fn seat_dir(&self, seat: &SeatId) -> PathBuf {
        self.seats_root.join(seat.to_string())
    }

    /// I/O for a seat that already has a stored session id.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the session id file is missing.
    pub async fn io_for(&self, binding: &NodeBinding) -> Result<ClaudeIo, Error> {
        let id = read_id(&self.seat_dir(&binding.seat)).await?;
        Ok(self.io(binding, id))
    }

    fn io(&self, binding: &NodeBinding, session_id: String) -> ClaudeIo {
        ClaudeIo::new(
            Arc::clone(&self.mux),
            Arc::clone(&self.check),
            crate::claude::submit::IoTarget {
                pane: binding.pane.clone(),
                home: self.home.clone(),
                cwd: binding.cwd.clone(),
                session_id,
            },
            self.submit_wait,
        )
    }
}

#[async_trait]
impl AgentAdapter for ClaudeAdapter {
    fn runtime(&self) -> RuntimeKind {
        RuntimeKind::ClaudeCode
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            structured_input: false,
            hooks: true,
            compaction_control: false,
            fork: false,
            usage_telemetry: false,
        }
    }

    async fn list_installed(
        &self,
        _binding: &NodeBinding,
    ) -> Result<Vec<InstalledResource>, Error> {
        Ok(Vec::new())
    }

    async fn project(
        &self,
        _plan: &ProjectionPlan,
        _binding: &NodeBinding,
    ) -> Result<ProjectionResult, Error> {
        Ok(ProjectionResult::default())
    }

    async fn deliver_startup(
        &self,
        _files: &[ResolvedStartupFile],
        _binding: &NodeBinding,
        _io: &dyn SeatIo,
    ) -> Result<StartupDeliveryResult, Error> {
        Ok(StartupDeliveryResult::default())
    }

    async fn launch(
        &self,
        binding: &NodeBinding,
        opts: LaunchOpts,
    ) -> Result<LaunchOutcome, Error> {
        if let Some(failed) = missing_cwd(binding).await? {
            return Ok(failed);
        }
        let seat_dir = self.seat_dir(&binding.seat);
        let prepared = launch::prepare(binding, &opts, &seat_dir).await?;
        let session_id = prepared.session_id.clone();
        let pane = self.mux.create_session(&prepared.spec).await?;
        let budget = opts.budget.unwrap_or_else(LaunchBudget::claude);
        let outcome = drive(
            Arc::clone(&self.mux),
            &pane,
            &binding.seat,
            opts.auto_trust,
            budget,
        )
        .await;
        self.store_force(binding.seat, outcome.force).await;
        Ok(LaunchOutcome::Ok {
            resume: Some(token(session_id)),
            applied: AppliedLaunch {
                tmux_session: pane.session.clone(),
                pane,
            },
        })
    }

    async fn check_ready(&self, binding: &NodeBinding) -> Result<Readiness, Error> {
        let capture = self
            .mux
            .capture(&binding.pane, CaptureRange::Visible)
            .await?;
        let live = readiness(&classify(&capture));
        if !matches!(live, Readiness::Inconclusive) {
            return Ok(live);
        }
        let guard = self.forced.lock().await;
        Ok(guard
            .get(&binding.seat)
            .cloned()
            .map_or(live, Readiness::Gate))
    }

    async fn discover_resume_token(
        &self,
        binding: &NodeBinding,
    ) -> Result<Option<ResumeToken>, Error> {
        match read_id(&self.seat_dir(&binding.seat)).await {
            Ok(id) => Ok(Some(token(id))),
            Err(Error::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
        }
    }
}

impl ClaudeAdapter {
    async fn store_force(&self, seat: SeatId, force: Option<ReadinessCode>) {
        let mut guard = self.forced.lock().await;
        match force {
            Some(code) => {
                guard.insert(seat, code);
            }
            None => {
                guard.remove(&seat);
            }
        }
    }
}

async fn missing_cwd(binding: &NodeBinding) -> Result<Option<LaunchOutcome>, Error> {
    let cwd = binding.cwd.clone();
    let exists = tokio::task::spawn_blocking(move || cwd.is_dir())
        .await
        .map_err(|err| Error::Message(err.to_string()))?;
    if exists {
        return Ok(None);
    }
    Ok(Some(LaunchOutcome::Failed {
        error: format!("cwd does not exist: {}", binding.cwd.display()),
        recovery: Some(crate::types::Recovery::AttentionRequired),
        evidence: Some(binding.cwd.display().to_string()),
    }))
}

async fn read_id(dir: &std::path::Path) -> Result<String, Error> {
    let text = tokio::fs::read_to_string(dir.join("claude-session-id")).await?;
    Ok(text.trim().to_owned())
}
