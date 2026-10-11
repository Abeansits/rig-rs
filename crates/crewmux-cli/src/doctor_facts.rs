//! Facts a doctor probe reports. Checks are pure functions of these facts.

/// Host operating system, as far as the Ghostty check cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostOs {
    /// macOS, where Ghostty is checked.
    Macos,
    /// Every other system. Ghostty is skipped.
    Other,
}

/// Codex `config.toml` as the doctor sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CodexView {
    /// The file is not there.
    Absent,
    /// The rig entry matches the command setup writes.
    Current,
    /// The file parses and the rig entry is missing.
    Missing,
    /// The rig entry is present with a different command.
    Drift,
    /// The file is not TOML. The message is a line and column, never the source line.
    Parse(String),
}

/// Herdr ping result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HerdrView {
    /// No Herdr binary.
    Absent,
    /// The binary is present and answered with `protocol`.
    Protocol(u32),
    /// The binary is present and did not answer.
    Unreachable,
}

/// Whether `$RIG_HOME` can hold the daemon socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HomeState {
    /// Writable, and the socket path fits in `sun_path`.
    Ok,
    /// The default path is too long, so a shorter fallback would be used.
    Fallback,
    /// The directory or its parent is not writable.
    Unwritable,
    /// An explicit `RIG_HOME` makes a socket path that does not fit.
    TooLong,
}

/// One tool the doctor looked up on `PATH`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolFact {
    /// The executable was found.
    pub present: bool,
    /// First line of `--version`, when the tool ran.
    pub version: String,
    /// Login or auth status, when the tool has one.
    pub logged_in: bool,
}

impl ToolFact {
    /// A tool that is not installed.
    #[must_use]
    pub(crate) fn missing() -> Self {
        Self {
            present: false,
            version: String::new(),
            logged_in: false,
        }
    }
}

/// Everything `rig doctor` needs, gathered without writing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DoctorFacts {
    /// Host used by the Ghostty check.
    pub os: HostOs,
    /// Raw `tmux -V` line, when tmux exists.
    pub tmux_version_line: Option<String>,
    /// Bytes of the user tmux config. Empty when the file is absent.
    pub tmux_conf: String,
    /// Effective global window size, when known. `manual` fails the check.
    pub window_size: Option<String>,
    /// `GET /healthz` on the daemon socket succeeded.
    pub daemon_up: bool,
    /// Claude CLI.
    pub claude: ToolFact,
    /// A Claude process is running for this user.
    pub claude_running: bool,
    /// Codex CLI.
    pub codex: ToolFact,
    /// Codex config classification.
    pub codex_config: CodexView,
    /// `OpenCode` CLI.
    pub opencode: ToolFact,
    /// Herdr ping.
    pub herdr: HerdrView,
    /// Ghostty CLI.
    pub ghostty: ToolFact,
    /// Ghostty is the Aqua front end.
    pub ghostty_aqua: bool,
    /// `$RIG_HOME` writability and socket length.
    pub rig_home: HomeState,
}
