// Derived from openrig packages/cli/src/commands/doctor.ts (v0.6.9, Apache-2.0); ported to Rust and modified.
//! `rig doctor`. Read-only: it does not create `$RIG_HOME` and it does not start the daemon.

use std::path::PathBuf;

use crewmux_types::CheckStatus;

use crate::command::{DoctorArgs, parse_providers};
use crate::doctor_eval::{DoctorOptions, evaluate, exit_code};
use crate::doctor_fixture::load_fixture;
use crate::doctor_live::gather;
use crate::error::Error;
use crate::output::print_stdout;
use crate::paths::Paths;

/// Run doctor and return the process status.
///
/// # Errors
///
/// Returns [`Error`] when the provider list or a fixture file is invalid.
pub(crate) fn run(paths: &Paths, args: &DoctorArgs) -> Result<i32, Error> {
    let providers = parse_providers(args.providers.as_deref())?;
    let facts = match fixture_path() {
        Some(path) => load_fixture(&path)?,
        None => gather(paths),
    };
    let report = evaluate(
        &facts,
        &DoctorOptions {
            providers,
            herdr: !args.no_herdr,
        },
    );
    if args.json {
        let body =
            serde_json::to_string_pretty(&report).map_err(|err| Error::Fixture(err.to_string()))?;
        print_stdout(&body);
    } else {
        print_text(&report.checks);
    }
    Ok(exit_code(&report))
}

fn fixture_path() -> Option<PathBuf> {
    let value = std::env::var_os("RIG_DOCTOR_FIXTURE")?;
    if value.is_empty() {
        None
    } else {
        Some(PathBuf::from(value))
    }
}

fn print_text(checks: &[crewmux_types::DoctorCheck]) {
    for check in checks {
        print_stdout(&format!(
            "{} {} {}",
            status_word(check.status),
            check.name,
            check.message
        ));
        if let Some(fix) = &check.fix {
            print_stdout(&format!("  fix: {fix}"));
        }
    }
}

fn status_word(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Pass => "pass",
        CheckStatus::Warn => "warn",
        CheckStatus::Fail => "fail",
        CheckStatus::Skipped => "skipped",
    }
}
