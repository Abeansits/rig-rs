//! Argument types for `rig setup` and `rig doctor`.
//!
//! The binary parser lives in `lib.rs` so `up`, `ps`, `setup`, and `doctor`
//! share one clap command tree.

use std::collections::BTreeSet;

use clap::Parser;

use crate::error::Error;

/// Flags for `rig setup`.
#[derive(Debug, Parser)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each field is a separate command-line switch"
)]
pub(crate) struct SetupArgs {
    /// Print the plan and write nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Print the plan as JSON.
    #[arg(long)]
    pub json: bool,
    /// Comma-separated providers. The default is `claude,codex,opencode`.
    #[arg(long)]
    pub providers: Option<String>,
    /// Leave Herdr out of the plan.
    #[arg(long)]
    pub no_herdr: bool,
    /// Offer the Herdr install. It runs only together with `--yes` or a prompt.
    #[arg(long)]
    pub herdr: bool,
    /// Offer a Ghostty install.
    #[arg(long)]
    pub ghostty: bool,
    /// Leave Ghostty out of the plan.
    #[arg(long)]
    pub no_ghostty: bool,
    /// Confirm install steps without a prompt.
    #[arg(long)]
    pub yes: bool,
}

/// Flags for `rig doctor`.
#[derive(Debug, Parser)]
pub(crate) struct DoctorArgs {
    /// Print the report as JSON.
    #[arg(long)]
    pub json: bool,
    /// Comma-separated providers. The default is `claude,codex,opencode`.
    #[arg(long)]
    pub providers: Option<String>,
    /// Skip the Herdr check.
    #[arg(long)]
    pub no_herdr: bool,
}

/// Which provider writers and checks are selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Providers {
    /// Claude seat files and the Claude check.
    pub claude: bool,
    /// Codex config and the Codex checks.
    pub codex: bool,
    /// Per-seat `OpenCode` config and the `OpenCode` check.
    pub opencode: bool,
}

impl Providers {
    /// `claude`, `codex`, and `opencode`.
    #[must_use]
    pub(crate) fn all() -> Self {
        Self {
            claude: true,
            codex: true,
            opencode: true,
        }
    }
}

/// Parse a comma-separated provider list.
///
/// # Errors
///
/// Returns [`Error::Usage`] when a name is not one of the three providers.
pub(crate) fn parse_providers(text: Option<&str>) -> Result<Providers, Error> {
    let Some(text) = text else {
        return Ok(Providers::all());
    };
    let mut seen = BTreeSet::new();
    for part in text.split(',') {
        let name = part.trim();
        if name.is_empty() {
            continue;
        }
        if !matches!(name, "claude" | "codex" | "opencode") {
            return Err(Error::Usage(format!("unknown provider {name}")));
        }
        seen.insert(name.to_owned());
    }
    if seen.is_empty() {
        return Err(Error::Usage(
            "pass at least one of claude, codex, opencode".to_owned(),
        ));
    }
    Ok(Providers {
        claude: seen.contains("claude"),
        codex: seen.contains("codex"),
        opencode: seen.contains("opencode"),
    })
}
