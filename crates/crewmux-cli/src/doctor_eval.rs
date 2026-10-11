// Derived from openrig packages/cli/src/commands/doctor.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
//! Pure doctor checks. Names and order match the slice 7 table.

use crewmux_provider::{classify_config, global_window_size_manual, managed_block_current};
use crewmux_types::{CheckStatus, DoctorCheck, parse_version};

use crate::command::Providers;
use crate::doctor_facts::{CodexView, DoctorFacts, HerdrView, HomeState, HostOs};

/// Options that change which checks run.
#[derive(Debug, Clone)]
pub(crate) struct DoctorOptions {
    /// Selected providers.
    pub providers: Providers,
    /// When false, the Herdr check is skipped.
    pub herdr: bool,
}

/// JSON body of `rig doctor --json`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct DoctorReport {
    /// True when no check failed.
    pub healthy: bool,
    /// Checks in table order.
    pub checks: Vec<DoctorCheck>,
}

/// Run every check.
#[must_use]
pub(crate) fn evaluate(facts: &DoctorFacts, options: &DoctorOptions) -> DoctorReport {
    let checks = vec![
        check_tmux(facts),
        check_tmux_conf(facts),
        check_window_size(facts),
        check_daemon(facts),
        check_claude(facts, options.providers.claude),
        check_claude_config(facts),
        check_codex(facts, options.providers.codex),
        check_codex_config(facts, options.providers.codex),
        check_opencode(facts, options.providers.opencode),
        check_herdr(facts, options.herdr),
        check_ghostty(facts),
        check_rig_home(facts),
    ];
    DoctorReport {
        healthy: !checks.iter().any(|check| check.status == CheckStatus::Fail),
        checks,
    }
}

/// Process status: `0` when [`DoctorReport::healthy`], otherwise `1`.
#[must_use]
pub(crate) fn exit_code(report: &DoctorReport) -> i32 {
    i32::from(!report.healthy)
}

fn check_tmux(facts: &DoctorFacts) -> DoctorCheck {
    let Some(line) = facts.tmux_version_line.as_deref() else {
        return DoctorCheck::fail("tmux", "tmux is not installed", "Install tmux 3.3 or newer");
    };
    match parse_version(line) {
        Ok(version) if version.is_supported() => {
            DoctorCheck::pass("tmux", format!("tmux {version}"))
        }
        Ok(version) => DoctorCheck::fail(
            "tmux",
            format!("tmux {version} is older than 3.3"),
            "Install tmux 3.3 or newer",
        ),
        Err(_) => DoctorCheck::fail(
            "tmux",
            format!("unrecognised tmux version line: {line}"),
            "Install tmux 3.3 or newer",
        ),
    }
}

fn check_tmux_conf(facts: &DoctorFacts) -> DoctorCheck {
    if facts.tmux_conf.is_empty() {
        return DoctorCheck::warn(
            "tmux_conf",
            "managed block is absent",
            "Run rig setup to install the tmux managed block",
        );
    }
    if managed_block_current(&facts.tmux_conf) {
        DoctorCheck::pass("tmux_conf", "managed block is current")
    } else {
        DoctorCheck::warn(
            "tmux_conf",
            "managed block drifted",
            "Run rig setup to refresh the tmux managed block",
        )
    }
}

fn check_window_size(facts: &DoctorFacts) -> DoctorCheck {
    // A failed `tmux show` (no server) is not evidence the setting is safe.
    // `window-size manual` crashes tmux 3.5a, so this does not start a server
    // to re-read the option. It scans the config the failed query left unread.
    let manual = match facts.window_size.as_deref() {
        Some("manual") => true,
        Some(_) => false,
        None => global_window_size_manual(scanned_tmux(facts)),
    };
    if manual {
        DoctorCheck::fail(
            "tmux_global_window_size",
            "global window-size is manual",
            "Remove the global window-size setting from the tmux config",
        )
    } else {
        DoctorCheck::pass(
            "tmux_global_window_size",
            "global window-size is not manual",
        )
    }
}

fn scanned_tmux(facts: &DoctorFacts) -> &str {
    if facts.tmux_scan.is_empty() {
        &facts.tmux_conf
    } else {
        &facts.tmux_scan
    }
}

fn check_daemon(facts: &DoctorFacts) -> DoctorCheck {
    if facts.daemon_up {
        DoctorCheck::pass("daemon", "daemon /healthz ok")
    } else {
        DoctorCheck::warn(
            "daemon",
            "daemon is not running",
            "Start the daemon when you need it; doctor does not start it",
        )
    }
}

fn check_claude(facts: &DoctorFacts, selected: bool) -> DoctorCheck {
    if !selected {
        return DoctorCheck::skipped("claude", "claude is not selected");
    }
    if !facts.claude.present {
        return DoctorCheck::fail(
            "claude",
            "claude is not installed",
            "Install the Claude CLI, then rerun rig doctor",
        );
    }
    if facts.claude.logged_in {
        DoctorCheck::pass("claude", tool_message("claude", &facts.claude.version))
    } else {
        DoctorCheck::warn(
            "claude",
            "claude is not logged in",
            "Run claude auth login, then rerun rig doctor",
        )
    }
}

fn check_claude_config(facts: &DoctorFacts) -> DoctorCheck {
    if facts.claude_running {
        DoctorCheck::warn(
            "claude_global_config",
            "trust pre-accept was skipped because Claude is running",
            "Quit Claude, then accept the trust prompt once",
        )
    } else {
        DoctorCheck::pass("claude_global_config", "no runtime writes pending")
    }
}

fn check_codex(facts: &DoctorFacts, selected: bool) -> DoctorCheck {
    if !selected {
        return DoctorCheck::skipped("codex", "codex is not selected");
    }
    if !facts.codex.present {
        return DoctorCheck::fail(
            "codex",
            "codex is not installed",
            "Install the Codex CLI, then rerun rig doctor",
        );
    }
    if facts.codex.logged_in {
        DoctorCheck::pass("codex", tool_message("codex", &facts.codex.version))
    } else {
        DoctorCheck::warn(
            "codex",
            "codex is not logged in",
            "Run codex login, then rerun rig doctor",
        )
    }
}

fn check_codex_config(facts: &DoctorFacts, selected: bool) -> DoctorCheck {
    if !selected {
        return DoctorCheck::skipped("codex_config", "codex is not selected");
    }
    match &facts.codex_config {
        CodexView::Absent => DoctorCheck::pass("codex_config", "codex config is absent"),
        CodexView::Current => DoctorCheck::pass("codex_config", "codex config entries are current"),
        CodexView::Missing => DoctorCheck::warn(
            "codex_config",
            "rig mcp entry is missing",
            "Rerun rig setup to add the rig mcp server",
        ),
        CodexView::Drift => DoctorCheck::warn(
            "codex_config",
            "codex config drifted from the rig mcp entry",
            "Rerun rig setup, or edit the Codex config so the rig server matches",
        ),
        CodexView::Parse(message) => DoctorCheck::fail(
            "codex_config",
            format!("codex config failed to parse: {message}"),
            "Repair the Codex config and rerun rig doctor",
        ),
    }
}

fn check_opencode(facts: &DoctorFacts, selected: bool) -> DoctorCheck {
    if !selected {
        return DoctorCheck::skipped("opencode", "opencode is not selected");
    }
    if !facts.opencode.present {
        return DoctorCheck::fail(
            "opencode",
            "opencode is not installed",
            "Install opencode 1.19.0",
        );
    }
    if opencode_version_ok(&facts.opencode.version) {
        DoctorCheck::pass(
            "opencode",
            tool_message("opencode", &facts.opencode.version),
        )
    } else {
        DoctorCheck::warn(
            "opencode",
            format!(
                "opencode {} is outside the tested 1.19 line",
                facts.opencode.version
            ),
            "Install opencode 1.19.0",
        )
    }
}

fn check_herdr(facts: &DoctorFacts, enabled: bool) -> DoctorCheck {
    if !enabled {
        return DoctorCheck::skipped("herdr", "herdr checks are disabled");
    }
    match facts.herdr {
        HerdrView::Absent => DoctorCheck::skipped("herdr", "herdr is not installed"),
        HerdrView::Unreachable => DoctorCheck::warn(
            "herdr",
            "herdr did not answer a ping",
            "Start herdr, or pass --no-herdr",
        ),
        HerdrView::Protocol(22) => DoctorCheck::pass("herdr", "herdr protocol 22"),
        HerdrView::Protocol(protocol) => DoctorCheck::warn(
            "herdr",
            format!("herdr protocol {protocol}, expected 22"),
            "Install a Herdr build that speaks protocol 22",
        ),
    }
}

fn check_ghostty(facts: &DoctorFacts) -> DoctorCheck {
    if facts.os != HostOs::Macos {
        return DoctorCheck::skipped("ghostty", "ghostty is skipped off macOS");
    }
    if !facts.ghostty.present {
        return DoctorCheck::fail(
            "ghostty",
            "ghostty is not installed",
            "Install Ghostty 1.3 or newer",
        );
    }
    if !version_at_least(&facts.ghostty.version, 1, 3) {
        return DoctorCheck::fail(
            "ghostty",
            format!("ghostty {} is older than 1.3", facts.ghostty.version),
            "Install Ghostty 1.3 or newer",
        );
    }
    if facts.ghostty_aqua {
        DoctorCheck::pass("ghostty", tool_message("ghostty", &facts.ghostty.version))
    } else {
        DoctorCheck::fail(
            "ghostty",
            "ghostty is not the Aqua front end",
            "Launch Ghostty as a macOS Aqua app",
        )
    }
}

fn check_rig_home(facts: &DoctorFacts) -> DoctorCheck {
    match facts.rig_home {
        HomeState::Ok => {
            DoctorCheck::pass("rig_home", "rig home is writable and the socket path fits")
        }
        HomeState::Fallback => DoctorCheck::warn(
            "rig_home",
            "the default socket path is too long; a fallback directory is in use",
            "Set RIG_HOME to a shorter path",
        ),
        HomeState::Unwritable => DoctorCheck::fail(
            "rig_home",
            "rig home is not writable",
            "Choose a writable RIG_HOME",
        ),
        HomeState::TooLong => DoctorCheck::fail(
            "rig_home",
            "RIG_HOME socket path exceeds the sun_path limit",
            "Set RIG_HOME to a shorter path",
        ),
    }
}

fn opencode_version_ok(version: &str) -> bool {
    let text = version_token(version);
    text == "1.19.0" || text.starts_with("1.19.")
}

fn version_at_least(version: &str, major: u32, minor: u32) -> bool {
    let text = version_token(version);
    let Some((major_s, rest)) = text.split_once('.') else {
        return false;
    };
    let minor_s = rest.split('.').next().unwrap_or(rest);
    let Ok(found_major) = major_s.parse::<u32>() else {
        return false;
    };
    let Ok(found_minor) = minor_s.parse::<u32>() else {
        return false;
    };
    (found_major, found_minor) >= (major, minor)
}

fn version_token(version: &str) -> &str {
    let text = version.split_whitespace().last().unwrap_or(version);
    text.trim_start_matches('v')
}

/// Classify Codex text the same way the live probe does.
#[must_use]
pub(crate) fn codex_view_from_text(text: Option<&str>) -> CodexView {
    let Some(text) = text else {
        return CodexView::Absent;
    };
    if text.trim().is_empty() {
        return CodexView::Absent;
    }
    match classify_config(text) {
        Ok(crewmux_provider::CodexHealth::Current) => CodexView::Current,
        Ok(crewmux_provider::CodexHealth::Missing) => CodexView::Missing,
        Ok(crewmux_provider::CodexHealth::Drift) => CodexView::Drift,
        Err(error) => CodexView::Parse(error.to_string()),
    }
}

fn tool_message(name: &str, version: &str) -> String {
    format!("{name} {}", version_token(version))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Providers;
    use crate::doctor_facts::{CodexView, DoctorFacts, HerdrView, HomeState, HostOs, ToolFact};

    fn tool(version: &str) -> ToolFact {
        ToolFact {
            present: true,
            version: version.to_owned(),
            logged_in: true,
        }
    }

    fn passing() -> DoctorFacts {
        DoctorFacts {
            os: HostOs::Macos,
            tmux_version_line: Some("tmux 3.5a".to_owned()),
            tmux_conf: crewmux_provider::render_tmux_block(),
            tmux_scan: String::new(),
            window_size: Some("latest".to_owned()),
            daemon_up: true,
            claude: tool("2.1.0"),
            claude_running: false,
            codex: tool("0.40.0"),
            codex_config: CodexView::Current,
            opencode: tool("1.19.0"),
            herdr: HerdrView::Protocol(22),
            ghostty: tool("1.3.0"),
            ghostty_aqua: true,
            rig_home: HomeState::Ok,
        }
    }

    #[test]
    fn problems_include_a_fix_hint() {
        let options = DoctorOptions {
            providers: Providers::all(),
            herdr: true,
        };
        let mut facts = passing();
        facts.tmux_version_line = Some("tmux 3.2".to_owned());
        facts.codex.logged_in = false;
        facts.opencode.version = "1.18.0".to_owned();
        facts.herdr = HerdrView::Protocol(21);
        facts.os = HostOs::Other;
        let report = evaluate(&facts, &options);
        for check in &report.checks {
            if matches!(check.status, CheckStatus::Fail | CheckStatus::Warn) {
                assert!(check.fix.is_some(), "{}", check.name);
            }
        }
        assert!(!report.healthy);
        assert_eq!(exit_code(&report), 1);
    }

    #[test]
    fn parse_error_omits_the_secret_and_names_the_line() {
        let secret = "token = \"sk-live-secret\"\nthis = [\n";
        let view = codex_view_from_text(Some(secret));
        let CodexView::Parse(message) = &view else {
            panic!("expected a parse failure");
        };
        assert!(!message.contains("sk-"), "{message}");
        assert!(message.contains("line "), "{message}");
        assert!(message.contains("column "), "{message}");
        let mut facts = passing();
        facts.codex_config = view;
        let report = evaluate(
            &facts,
            &DoctorOptions {
                providers: Providers::all(),
                herdr: true,
            },
        );
        let rendered = format!("{report:?}");
        assert!(!rendered.contains("sk-"), "{rendered}");
    }

    #[test]
    fn missing_rig_mcp_entry_warns_and_version_is_not_doubled() {
        let mut facts = passing();
        facts.codex_config = CodexView::Missing;
        facts.claude.version = "claude 1.0.0".to_owned();
        let report = evaluate(
            &facts,
            &DoctorOptions {
                providers: Providers::all(),
                herdr: true,
            },
        );
        let codex = report
            .checks
            .iter()
            .find(|check| check.name == "codex_config")
            .expect("codex_config");
        assert_eq!(codex.status, CheckStatus::Warn);
        assert!(codex.fix.is_some());
        let claude = report
            .checks
            .iter()
            .find(|check| check.name == "claude")
            .expect("claude");
        assert_eq!(claude.message, "claude 1.0.0");
    }

    #[test]
    fn no_server_manual_conf_fails_window_size() {
        let options = DoctorOptions {
            providers: Providers::all(),
            herdr: true,
        };
        let mut facts = passing();
        facts.window_size = None;
        facts.tmux_conf = "set -g window-size manual\n".to_owned();
        let failed = evaluate(&facts, &options);
        let check = failed
            .checks
            .iter()
            .find(|check| check.name == "tmux_global_window_size")
            .expect("tmux_global_window_size");
        assert_eq!(check.status, CheckStatus::Fail);
        assert!(check.fix.is_some());

        facts.tmux_conf = "set -g mouse on\n".to_owned();
        let clean = evaluate(&facts, &options);
        let passed = clean
            .checks
            .iter()
            .find(|check| check.name == "tmux_global_window_size")
            .expect("tmux_global_window_size");
        assert_eq!(passed.status, CheckStatus::Pass);

        facts.window_size = Some("latest".to_owned());
        facts.tmux_conf = "set -g window-size manual\n".to_owned();
        let live = evaluate(&facts, &options);
        let from_server = live
            .checks
            .iter()
            .find(|check| check.name == "tmux_global_window_size")
            .expect("tmux_global_window_size");
        assert_eq!(from_server.status, CheckStatus::Pass);
    }

    #[test]
    fn tmux_missing_fails_with_a_fix() {
        let mut facts = passing();
        facts.tmux_version_line = None;
        let report = evaluate(
            &facts,
            &DoctorOptions {
                providers: Providers::all(),
                herdr: true,
            },
        );
        let tmux = report
            .checks
            .iter()
            .find(|check| check.name == "tmux")
            .expect("tmux");
        assert_eq!(tmux.status, CheckStatus::Fail);
        assert!(tmux.fix.is_some());
    }
}
