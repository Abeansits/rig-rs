//! Agent runtime adapters. Slice 1 ships the trait and the stub runtime.

mod adapter;
pub mod codex;
pub mod dialog;
mod error;
mod readiness;
mod registry;
mod stub;
mod types;

pub use adapter::{AgentAdapter, SeatIo};
pub use codex::CodexAdapter;
pub use error::Error;
pub use readiness::{ReadinessWatch, WatchDecision, default_budget, default_launch_timeout, stamp};
pub use registry::AdapterRegistry;
pub use stub::{StubAdapter, StubScript, binding_for, classify, parse_script, run_stub_agent};
pub use types::{
    AppliedLaunch, Capabilities, DeliveryReceipt, InstalledResource, LaunchOpts, LaunchOutcome,
    NodeBinding, ProjectionPlan, ProjectionResult, Readiness, Recovery, ResolvedStartupFile,
    ResumeToken, StartupDeliveryResult, SubmitMode,
};

#[cfg(test)]
mod screen_tests {
    use crewmux_types::ReadinessCode;

    use crate::stub::{classify, parse_script};
    use crate::types::Readiness;

    #[test]
    fn ready_prompt_is_ready() {
        let verdict = classify("stub-agent\nstub> ");
        assert_eq!(verdict, Readiness::Ready);
    }

    #[test]
    fn gate_is_attention_even_with_a_prompt() {
        let verdict = classify("stub gate: login_required\nstub> ");
        assert_eq!(verdict, Readiness::Gate(ReadinessCode::LoginRequired));
        assert!(ReadinessCode::LoginRequired.requires_attention());
    }

    #[test]
    fn static_banner_is_inconclusive() {
        assert_eq!(
            classify("stub-agent\nstill starting\n"),
            Readiness::Inconclusive
        );
    }

    #[test]
    fn script_parses_gate_and_timers() {
        let script = parse_script("ready_after_ms=10,exit_after_ms=20,print_gate=login_required")
            .expect("script");
        assert_eq!(script.ready_after_ms, 10);
        assert_eq!(script.exit_after_ms, Some(20));
        assert_eq!(script.print_gate.as_deref(), Some("login_required"));
        assert!(!script.screen().contains("stub> "));
        assert!(script.screen().contains("stub gate: login_required"));
    }
}
