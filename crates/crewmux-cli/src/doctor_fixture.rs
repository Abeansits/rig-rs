//! `RIG_DOCTOR_FIXTURE` loads facts from JSON so goldens do not read the host.

use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::doctor_facts::{CodexView, DoctorFacts, HerdrView, HomeState, HostOs, ToolFact};
use crate::error::Error;

#[derive(Debug, Deserialize)]
struct Fixture {
    os: String,
    #[serde(default)]
    tmux_version: Option<String>,
    #[serde(default)]
    tmux_conf: String,
    #[serde(default)]
    window_size: Option<String>,
    daemon_up: bool,
    claude: ToolFixture,
    #[serde(default)]
    claude_running: bool,
    codex: ToolFixture,
    codex_config: String,
    opencode: ToolFixture,
    herdr: String,
    ghostty: ToolFixture,
    #[serde(default)]
    ghostty_aqua: bool,
    rig_home: String,
}

#[derive(Debug, Deserialize)]
struct ToolFixture {
    present: bool,
    #[serde(default)]
    version: String,
    #[serde(default)]
    logged_in: bool,
}

/// Load a fixture file.
///
/// # Errors
///
/// Returns [`Error::Fixture`] when the file or a field is invalid.
pub(crate) fn load_fixture(path: &Path) -> Result<DoctorFacts, Error> {
    let text = fs::read_to_string(path).map_err(|err| Error::Fixture(err.to_string()))?;
    let fixture: Fixture =
        serde_json::from_str(&text).map_err(|err| Error::Fixture(err.to_string()))?;
    Ok(DoctorFacts {
        os: parse_os(&fixture.os)?,
        tmux_version_line: fixture.tmux_version,
        tmux_conf: fixture.tmux_conf,
        window_size: fixture.window_size,
        daemon_up: fixture.daemon_up,
        claude: tool_from(fixture.claude),
        claude_running: fixture.claude_running,
        codex: tool_from(fixture.codex),
        codex_config: parse_codex(&fixture.codex_config)?,
        opencode: tool_from(fixture.opencode),
        herdr: parse_herdr(&fixture.herdr)?,
        ghostty: tool_from(fixture.ghostty),
        ghostty_aqua: fixture.ghostty_aqua,
        rig_home: parse_home(&fixture.rig_home)?,
    })
}

fn tool_from(tool: ToolFixture) -> ToolFact {
    ToolFact {
        present: tool.present,
        version: tool.version,
        logged_in: tool.logged_in,
    }
}

fn parse_os(text: &str) -> Result<HostOs, Error> {
    match text {
        "macos" => Ok(HostOs::Macos),
        "linux" | "other" => Ok(HostOs::Other),
        other => Err(Error::Fixture(format!("unknown os {other}"))),
    }
}

fn parse_codex(text: &str) -> Result<CodexView, Error> {
    match text {
        "absent" => Ok(CodexView::Absent),
        "current" => Ok(CodexView::Current),
        "drift" => Ok(CodexView::Drift),
        "parse" => Ok(CodexView::Parse("invalid TOML".to_owned())),
        other => Err(Error::Fixture(format!("unknown codex_config {other}"))),
    }
}

fn parse_herdr(text: &str) -> Result<HerdrView, Error> {
    match text {
        "absent" => Ok(HerdrView::Absent),
        "unreachable" => Ok(HerdrView::Unreachable),
        "22" => Ok(HerdrView::Protocol(22)),
        "21" => Ok(HerdrView::Protocol(21)),
        other => Err(Error::Fixture(format!("unknown herdr {other}"))),
    }
}

fn parse_home(text: &str) -> Result<HomeState, Error> {
    match text {
        "ok" => Ok(HomeState::Ok),
        "fallback" => Ok(HomeState::Fallback),
        "unwritable" => Ok(HomeState::Unwritable),
        "too_long" => Ok(HomeState::TooLong),
        other => Err(Error::Fixture(format!("unknown rig_home {other}"))),
    }
}
