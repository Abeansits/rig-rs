//! Stub runtime: screen classification, the env script, and the fixture process.

mod buffer;
mod run;
mod screen;
mod script;

pub use run::run_stub_agent;
pub use screen::classify;
pub use script::{StubScript, parse_script};

use std::sync::Arc;

use async_trait::async_trait;
use crewmux_mux::{CaptureRange, Multiplexer, PaneRef, SessionSpec};
use crewmux_types::{LaunchPosture, RuntimeKind};

use crate::adapter::{AgentAdapter, SeatIo};
use crate::error::Error;
use crate::types::{
    AppliedLaunch, Capabilities, InstalledResource, LaunchOpts, LaunchOutcome, NodeBinding,
    ProjectionPlan, ProjectionResult, Readiness, ResolvedStartupFile, ResumeToken,
    StartupDeliveryResult,
};

/// Launches `rig-stub-agent` (or a fake multiplexer that honours the same script).
pub struct StubAdapter {
    mux: Arc<dyn Multiplexer>,
    binary: std::path::PathBuf,
}

impl StubAdapter {
    /// `binary` is the fixture path. [`crewmux_mux::FakeMux`] does not execute it.
    #[must_use]
    pub fn new(mux: Arc<dyn Multiplexer>, binary: impl Into<std::path::PathBuf>) -> Self {
        Self {
            mux,
            binary: binary.into(),
        }
    }
}

#[async_trait]
impl AgentAdapter for StubAdapter {
    fn runtime(&self) -> RuntimeKind {
        RuntimeKind::Stub
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
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
        let cwd = binding.cwd.clone();
        let exists = tokio::task::spawn_blocking(move || cwd.is_dir())
            .await
            .map_err(|err| Error::Message(err.to_string()))?;
        if !exists {
            return Ok(LaunchOutcome::Failed {
                error: format!("cwd does not exist: {}", binding.cwd.display()),
                recovery: Some(crate::types::Recovery::AttentionRequired),
                evidence: Some(binding.cwd.display().to_string()),
            });
        }
        let mut spec = SessionSpec::new(opts.session_name.clone(), binding.cwd.clone());
        spec.env = opts.env;
        spec.argv = vec![self.binary.display().to_string()];
        spec.geometry = crewmux_mux::Geometry { cols: 90, rows: 27 };
        spec.remain_on_exit = false;
        let pane = self.mux.create_session(&spec).await?;
        Ok(LaunchOutcome::Ok {
            resume: None,
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
        Ok(classify(&capture.text))
    }

    async fn discover_resume_token(
        &self,
        _binding: &NodeBinding,
    ) -> Result<Option<ResumeToken>, Error> {
        Ok(None)
    }
}

/// Empty binding used before a pane exists.
#[must_use]
pub fn binding_for(
    seat: crewmux_types::SeatId,
    cwd: std::path::PathBuf,
    session: &str,
) -> NodeBinding {
    NodeBinding {
        seat,
        cwd,
        model: None,
        effort: None,
        posture: LaunchPosture::Auto,
        permission_mode: None,
        non_interruptive: true,
        pane: PaneRef::session(session),
        launch_generation: "1".to_owned(),
        runtime_opts: serde_json::Value::Null,
    }
}
