//! Differential harness: hermetic homes, normalisation, goldens, and verdicts.

mod bind;
mod cases;
mod compare;
mod golden;
mod hermetic;
mod mcp_list;
mod normalize;
mod openrig;
mod reap;
mod report;
mod rig;
mod run;
mod suite;
mod types;

pub use bind::{bind_transport, covers_for, pe_ids_in, tcp_target};
pub use cases::{load_case, load_dir, load_suite};
pub use compare::{ExpectedScope, help_command_names, judge, project, unified_diff};
pub use golden::{read_rs_golden, read_ts_golden, write_ts_golden};
pub use hermetic::{ambient_clock, ambient_tmux, foreign_daemon_target};
pub use mcp_list::map_tool;
pub use normalize::{normalize_observation, normalize_text};
pub use openrig::{find_node, install_instructions, locate_openrig};
pub use report::{CaseOutcome, CaseReport, exit_code, format_report};
pub use run::{Executed, execute, execute_rs};
pub use suite::{self_reports, ts_reports};
pub use types::{
    DiffCase, DiffVerdict, HermeticHome, LoadedCase, Method, NormRule, OPENRIG_COMMIT, OPENRIG_TAG,
    ObsField, Observation, PeId, Project, Side, Step, Transport, canonical, workspace_root,
};

#[cfg(test)]
mod tests {
    use super::run::{chains_cli_json, json_body};
    use super::types::{DiffCase, Step};

    fn case(argv: &[&str]) -> DiffCase {
        DiffCase {
            id: "sample".to_owned(),
            steps: vec![Step::Cli {
                argv: argv.iter().map(|arg| (*arg).to_owned()).collect(),
            }],
            normalize: Vec::new(),
            expected_diff: None,
            covers: Vec::new(),
        }
    }

    #[test]
    fn one_document_is_chained_and_a_second_line_is_not() {
        let one = json_body("{\"ok\":true}\n");
        assert_eq!(
            one.as_ref().and_then(|value| value.get("ok")),
            Some(&serde_json::Value::Bool(true))
        );
        let pretty = json_body("{\n  \"ok\": true\n}\n");
        assert!(pretty.is_some());
        assert!(json_body("{\"a\":1}\n{\"b\":2}\n").is_none());
        assert!(chains_cli_json(&case(&[
            "workflow",
            "{{instance.instanceId}}"
        ])));
        assert!(!chains_cli_json(&case(&["--help"])));
    }
}
