//! [`AgentAdapter`]: one runtime, from launch through readiness.

use async_trait::async_trait;
use crewmux_mux::Capture;
use crewmux_types::RuntimeKind;

use crate::error::Error;
use crate::types::{
    Capabilities, DeliveryReceipt, InstalledResource, LaunchOpts, LaunchOutcome, NodeBinding,
    ProjectionPlan, ProjectionResult, Readiness, ResolvedStartupFile, ResumeToken,
    StartupDeliveryResult,
};

/// How text reaches an agent. Claude and Codex use the pane; the stub does not submit.
#[async_trait]
pub trait SeatIo: Send + Sync {
    /// Submit one turn.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the pane rejects the write.
    async fn submit(
        &self,
        text: &str,
        mode: crate::types::SubmitMode,
    ) -> Result<crate::types::DeliveryReceipt, Error>;

    /// Interrupt the current turn.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the pane cannot be interrupted.
    async fn interrupt(&self) -> Result<(), Error>;

    /// Capture the current screen.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when capture fails.
    async fn snapshot(&self) -> Result<Capture, Error>;
}

/// A runtime the daemon can launch.
#[async_trait]
pub trait AgentAdapter: Send + Sync + 'static {
    /// Which runtime this adapter owns.
    fn runtime(&self) -> RuntimeKind;

    /// Capability flags.
    fn capabilities(&self) -> Capabilities;

    /// Resources installed for this seat.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the runtime cannot be queried.
    async fn list_installed(&self, binding: &NodeBinding) -> Result<Vec<InstalledResource>, Error>;

    /// Project skills and instruction files. Slice 1 writes nothing.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when a write fails.
    async fn project(
        &self,
        plan: &ProjectionPlan,
        binding: &NodeBinding,
    ) -> Result<ProjectionResult, Error>;

    /// Hand startup files to the runtime.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when delivery fails.
    async fn deliver_startup(
        &self,
        files: &[ResolvedStartupFile],
        binding: &NodeBinding,
        io: &dyn SeatIo,
    ) -> Result<StartupDeliveryResult, Error>;

    /// Create the tmux session.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the multiplexer fails. A bad cwd is [`LaunchOutcome::Failed`].
    async fn launch(&self, binding: &NodeBinding, opts: LaunchOpts)
    -> Result<LaunchOutcome, Error>;

    /// Classify the current screen.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when capture fails.
    async fn check_ready(&self, binding: &NodeBinding) -> Result<Readiness, Error>;

    /// Submit one turn through the seat pane.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when this runtime has no submit path, or the pane rejects it.
    async fn submit(&self, _binding: &NodeBinding, _text: &str) -> Result<DeliveryReceipt, Error> {
        Err(Error::Message(
            "this runtime does not submit turns".to_owned(),
        ))
    }

    /// Provider resume token, when one is on disk.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the lookup fails.
    async fn discover_resume_token(
        &self,
        binding: &NodeBinding,
    ) -> Result<Option<ResumeToken>, Error>;
}
