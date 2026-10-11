// Derived from openrig packages/cli/src/commands/setup.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
//! `rig setup`. `--dry-run` prints the plan and does not create `$RIG_HOME`.

use std::io::IsTerminal;

use crate::command::{SetupArgs, parse_providers};
use crate::error::Error;
use crate::output::{print_stderr, print_stdout};
use crate::paths::Paths;
use crate::setup_apply::{ApplyCtx, apply, stdin_yes};
use crate::setup_plan::build_plan;

/// Run setup and return the process status.
///
/// # Errors
///
/// Returns [`Error`] when the provider list is invalid or an apply step fails.
pub(crate) fn run(paths: &Paths, args: &SetupArgs) -> Result<i32, Error> {
    let providers = parse_providers(args.providers.as_deref())?;
    let plan = build_plan(paths, args, &providers);
    if args.json || args.dry_run {
        let body =
            serde_json::to_string_pretty(&plan).map_err(|err| Error::Usage(err.to_string()))?;
        print_stdout(&body);
    }
    if args.dry_run {
        return Ok(0);
    }
    if !args.json {
        print_stdout("applying setup plan");
    }
    apply(
        &plan,
        &ApplyCtx {
            paths,
            confirm_installs: confirm(args),
            macos: std::env::consts::OS == "macos",
        },
    )?;
    Ok(0)
}

fn confirm(args: &SetupArgs) -> bool {
    if args.yes {
        return true;
    }
    if args.json || !std::io::stdin().is_terminal() {
        return false;
    }
    print_stderr("Install offered tools? [y/N]");
    stdin_yes()
}
