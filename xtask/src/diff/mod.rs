//! Differential harness: hermetic homes, normalisation, goldens, and verdicts.

mod bind;
mod cases;
mod compare;
mod golden;
mod hermetic;
mod normalize;
mod openrig;
mod report;
mod run;
mod suite;
mod types;

pub use bind::{bind_transport, pe_ids_in, tcp_target};
pub use cases::{load_case, load_dir, load_suite};
pub use compare::{help_command_names, judge, project, unified_diff};
pub use golden::{read_ts_golden, write_ts_golden};
pub use hermetic::{ambient_clock, ambient_tmux, foreign_daemon_target};
pub use normalize::{normalize_observation, normalize_text};
pub use openrig::{find_node, install_instructions, locate_openrig};
pub use report::{CaseOutcome, CaseReport, exit_code, format_report};
pub use run::{Executed, execute, execute_rs};
pub use suite::{self_reports, ts_reports};
pub use types::{
    DiffCase, DiffVerdict, HermeticHome, LoadedCase, Method, NormRule, OPENRIG_COMMIT, OPENRIG_TAG,
    Observation, PeId, Project, Side, Step, Transport, canonical, workspace_root,
};
