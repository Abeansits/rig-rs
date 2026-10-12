//! Each Claude launch gets a fresh per-run settings file on `--settings`.
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "temp-dir setup failures should fail the test"
)]

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crewmux_adapters::{
    AgentAdapter, BudgetOverrides, ClaudeAdapter, LaunchBudget, LaunchOpts, ResumeToken,
};
use crewmux_mux::FakeMux;
use crewmux_types::SeatId;
use serde_json::Value;

#[path = "common/mod.rs"]
mod support;

#[tokio::test]
async fn per_run_settings() {
    let root = scratch("run-settings");
    let cwd = root.join("work");
    std::fs::create_dir_all(&cwd).expect("cwd");
    let mux = Arc::new(FakeMux::new());
    let ready = support::read(&support::fixtures().join("2.1.286/claude-020-ready.txt"));
    mux.set_capture(support::capture(&ready)).await;
    let adapter = ClaudeAdapter::new(support::shared(&mux), root.join("seats"), root.join("home"));
    let binding = crewmux_adapters::binding_for(SeatId::new(), cwd, "seat__rig");
    adapter.launch(&binding, opts(None)).await.expect("fresh");
    adapter
        .launch(
            &binding,
            opts(Some(ResumeToken {
                kind: "claude".to_owned(),
                value: "11111111-1111-4111-8111-111111111111".to_owned(),
            })),
        )
        .await
        .expect("resume");
    let run = adapter.seat_dir(&binding.seat).join("run");
    let mut settings = Vec::new();
    for entry in std::fs::read_dir(&run).expect("run") {
        let path = entry.expect("entry").path().join("settings.json");
        assert!(path.is_file(), "{}", path.display());
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        settings.push(path);
    }
    assert_eq!(settings.len(), 2);
    let calls = mux.recorded().await;
    let flat: Vec<String> = calls.into_iter().flatten().collect();
    for path in &settings {
        let rendered = path.display().to_string();
        assert!(flat.iter().any(|arg| arg == &rendered), "{flat:?}");
        assert_hooks(&std::fs::read_to_string(path).expect("json"));
    }
    drop(std::fs::remove_dir_all(&root));
}

fn assert_hooks(body: &str) {
    let value: Value = serde_json::from_str(body).expect("settings");
    let hooks = value
        .get("hooks")
        .and_then(Value::as_object)
        .expect("hooks");
    for event in [
        "SessionStart",
        "UserPromptSubmit",
        "PreCompact",
        "PostCompact",
        "Stop",
        "SessionEnd",
    ] {
        let command = command_of(hooks.get(event));
        assert!(command.contains("RIG_SEAT_KEY="), "{command}");
        assert!(command.contains("RIG_HOME="), "{command}");
        assert!(command.contains("RIG_RUN_ID="), "{command}");
        assert!(command.contains(&format!(" hook {event}")), "{command}");
        assert!(
            command.contains('/'),
            "hook command must use an absolute rig path: {command}"
        );
    }
}

fn command_of(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_array)
        .and_then(|groups| groups.first())
        .and_then(|group| group.get("hooks"))
        .and_then(Value::as_array)
        .and_then(|hooks| hooks.first())
        .and_then(|hook| hook.get("command"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn opts(resume: Option<ResumeToken>) -> LaunchOpts {
    LaunchOpts {
        session_name: "seat__rig".to_owned(),
        env: BTreeMap::new(),
        resume,
        fork: false,
        auto_trust: false,
        budget: Some(LaunchBudget::with_overrides(BudgetOverrides {
            first_screen: Some(Duration::from_millis(20)),
            ready_after_last_gate: Some(Duration::from_millis(20)),
            inconclusive_stable: Some(Duration::from_millis(20)),
            overall: Some(Duration::from_millis(80)),
            poll: Some(Duration::from_millis(10)),
        })),
        claude_bin: None,
    }
}

fn scratch(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("rig-{label}-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&root));
    std::fs::create_dir_all(&root).expect("scratch");
    root
}
