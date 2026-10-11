//! Types the [`crate::AgentAdapter`] trait talks in.

use std::path::PathBuf;

use crewmux_mux::PaneRef;
use crewmux_types::{LaunchPosture, ReadinessCode, SeatId};
use serde_json::Value;

/// What a runtime can do. The stub reports every flag false.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "these five flags are the AgentAdapter capability sketch"
)]
pub struct Capabilities {
    /// The runtime accepts structured input.
    pub structured_input: bool,
    /// The runtime loads hooks.
    pub hooks: bool,
    /// The runtime can compact its transcript.
    pub compaction_control: bool,
    /// The runtime can fork a session.
    pub fork: bool,
    /// The runtime reports token usage.
    pub usage_telemetry: bool,
}

/// One installed skill or resource. The stub list is empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledResource {
    /// Resource name.
    pub name: String,
}

/// Files the daemon wants projected into a seat. Unused in slice 1.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectionPlan {
    /// Relative paths.
    pub files: Vec<String>,
}

/// What [`crate::AgentAdapter::project`] wrote.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectionResult {
    /// Paths that were written.
    pub written: Vec<String>,
}

/// A startup file the daemon has already resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedStartupFile {
    /// Path relative to the seat.
    pub path: String,
    /// File body.
    pub body: String,
}

/// How many startup files were delivered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StartupDeliveryResult {
    /// Count.
    pub delivered: usize,
}

/// How text is submitted. Slice 4 owns the guard; the stub does not submit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitMode {
    /// Paste, then Enter.
    Enter,
    /// Paste only.
    Paste,
}

/// Receipt for a submit. Slice 1 does not deliver turns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeliveryReceipt {
    /// Bytes accepted.
    pub bytes: usize,
}

/// A provider resume token. The stub never has one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResumeToken {
    /// Token family (`claude`, `codex`, …).
    pub kind: String,
    /// Token value.
    pub value: String,
}

/// What to do after a failed launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recovery {
    /// Try again without resume state.
    RetryFresh,
    /// Stop and surface attention.
    AttentionRequired,
}

/// Where the seat lives and how it should launch.
#[derive(Clone, Debug)]
pub struct NodeBinding {
    /// Seat id.
    pub seat: SeatId,
    /// Working directory.
    pub cwd: PathBuf,
    /// Model pin.
    pub model: Option<String>,
    /// Effort pin. Unused by the stub.
    pub effort: Option<String>,
    /// Permission floor.
    pub posture: LaunchPosture,
    /// Provider permission mode.
    pub permission_mode: Option<String>,
    /// When true, later slices must not paste over a human.
    pub non_interruptive: bool,
    /// Pane, filled in after a successful launch.
    pub pane: PaneRef,
    /// Generation string so a stale pane can be ignored.
    pub launch_generation: String,
    /// Adapter-specific options.
    pub runtime_opts: Value,
}

/// Options for [`crate::AgentAdapter::launch`].
#[derive(Clone, Debug)]
pub struct LaunchOpts {
    /// tmux session name (`seat__rig`).
    pub session_name: String,
    /// Extra environment (`RIG_STUB_SCRIPT`, …).
    pub env: std::collections::BTreeMap<String, String>,
    /// Resume token, when relaunching. The stub ignores it.
    pub resume: Option<ResumeToken>,
    /// Fork instead of a fresh session. The stub ignores it.
    pub fork: bool,
}

/// Pane and session created by a successful launch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppliedLaunch {
    /// Pane the multiplexer returned.
    pub pane: PaneRef,
    /// Canonical tmux session name.
    pub tmux_session: String,
}

/// Result of [`crate::AgentAdapter::launch`].
#[derive(Clone, Debug)]
pub enum LaunchOutcome {
    /// The session exists.
    Ok {
        /// Resume token, when the runtime reported one.
        resume: Option<ResumeToken>,
        /// Pane address.
        applied: AppliedLaunch,
    },
    /// The session was not created.
    Failed {
        /// Operator-facing reason.
        error: String,
        /// Suggested recovery.
        recovery: Option<Recovery>,
        /// Screen or path evidence.
        evidence: Option<String>,
    },
}

/// Screen classification shared by the stub, and later by Claude and Codex.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Readiness {
    /// Positive composer match.
    Ready,
    /// Still starting, no stable screen yet.
    Booting,
    /// A known gate. Ready is not implied.
    Gate(ReadinessCode),
    /// Neither a gate nor a composer.
    Inconclusive,
    /// The runtime cannot become ready.
    HardFailure(String),
}
