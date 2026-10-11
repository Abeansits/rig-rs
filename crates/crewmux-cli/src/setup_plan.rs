// Derived from openrig packages/cli/src/commands/setup.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
//! Plan-then-apply steps. Dry-run prints this value and writes nothing.

use serde::Serialize;

use crate::command::{Providers, SetupArgs};
use crate::paths::Paths;

/// One planned step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SetupStep {
    /// Create or update a file.
    Write {
        /// Stable id used by apply.
        id: String,
        /// Destination path.
        file: String,
        /// `upsert` when the writer will publish bytes.
        change: String,
    },
    /// Copy embedded skills into `$RIG_HOME/skills`.
    Extract {
        /// Destination directory.
        skills_dir: String,
    },
    /// An install that runs only after confirmation.
    OfferInstall {
        /// Tool name.
        tool: String,
        /// Shell command. It is not executed during planning.
        cmd: String,
    },
    /// A step that does not run.
    Skip {
        /// What was skipped.
        what: String,
        /// Why it was skipped.
        reason: String,
    },
}

/// The full plan `rig setup --dry-run --json` prints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SetupPlan {
    /// Steps in apply order.
    pub steps: Vec<SetupStep>,
}

/// Herdr's install script. Tests pass `--no-herdr` so this is never executed.
pub(crate) const HERDR_INSTALL: &str = "curl -fsSL https://herdr.dev/install.sh | sh";

/// Ghostty is installed with Homebrew on macOS only.
pub(crate) const GHOSTTY_INSTALL: &str = "brew install --cask ghostty";

/// Build the plan from flags and paths. This function does not touch the disk.
#[must_use]
pub(crate) fn build_plan(paths: &Paths, args: &SetupArgs, providers: &Providers) -> SetupPlan {
    let mut steps = Vec::new();
    push_provider(&mut steps, paths, providers);
    steps.push(write_step(
        "tmux_conf",
        paths.tmux_conf.display().to_string(),
    ));
    steps.push(SetupStep::Extract {
        skills_dir: paths.skills_dir().display().to_string(),
    });
    push_herdr(&mut steps, paths, args.no_herdr);
    push_ghostty(&mut steps, args);
    SetupPlan { steps }
}

fn push_provider(steps: &mut Vec<SetupStep>, paths: &Paths, providers: &Providers) {
    let seat = paths.seat_dir();
    if providers.claude {
        steps.push(write_step(
            "claude_mcp",
            seat.join("mcp.json").display().to_string(),
        ));
        steps.push(write_step(
            "claude_settings",
            seat.join("settings.json").display().to_string(),
        ));
    } else {
        steps.push(skip("claude", "not selected"));
    }
    if providers.codex {
        steps.push(write_step(
            "codex",
            paths.codex_config.display().to_string(),
        ));
    } else {
        steps.push(skip("codex", "not selected"));
    }
    if providers.opencode {
        steps.push(write_step(
            "opencode",
            seat.join("opencode.json").display().to_string(),
        ));
    } else {
        steps.push(skip("opencode", "not selected"));
    }
}

fn push_herdr(steps: &mut Vec<SetupStep>, paths: &Paths, disabled: bool) {
    if disabled {
        steps.push(skip("herdr", "disabled with --no-herdr"));
        return;
    }
    let personal = paths
        .herdr_config
        .clone()
        .unwrap_or_else(|| herdr_personal(paths));
    steps.push(write_step("herdr_launch", personal.display().to_string()));
    steps.push(SetupStep::OfferInstall {
        tool: "herdr".to_owned(),
        cmd: HERDR_INSTALL.to_owned(),
    });
}

fn herdr_personal(paths: &Paths) -> std::path::PathBuf {
    let root = paths
        .xdg_config_home
        .clone()
        .unwrap_or_else(|| paths.home.join(".config"));
    root.join("herdr").join("config.toml")
}

fn push_ghostty(steps: &mut Vec<SetupStep>, args: &SetupArgs) {
    if args.no_ghostty || !args.ghostty {
        let reason = if args.no_ghostty {
            "disabled with --no-ghostty"
        } else {
            "not requested"
        };
        steps.push(skip("ghostty", reason));
        return;
    }
    steps.push(SetupStep::OfferInstall {
        tool: "ghostty".to_owned(),
        cmd: GHOSTTY_INSTALL.to_owned(),
    });
}

fn write_step(id: &str, file: String) -> SetupStep {
    SetupStep::Write {
        id: id.to_owned(),
        file,
        change: "upsert".to_owned(),
    }
}

fn skip(what: &str, reason: &str) -> SetupStep {
    SetupStep::Skip {
        what: what.to_owned(),
        reason: reason.to_owned(),
    }
}
