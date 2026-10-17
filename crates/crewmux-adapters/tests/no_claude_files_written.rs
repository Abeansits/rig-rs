//! Launch does not create or modify the user or project Claude config.
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "temp-dir setup failures should fail the test"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crewmux_adapters::{AgentAdapter, BudgetOverrides, ClaudeAdapter, LaunchBudget, LaunchOpts};
use crewmux_mux::FakeMux;
use crewmux_types::SeatId;

#[path = "common/mod.rs"]
mod support;

#[tokio::test]
async fn no_claude_files_written() {
    let root = scratch("no-claude-files");
    let cwd = root.join("work");
    let home = root.join("home");
    std::fs::create_dir_all(home.join(".claude")).expect("claude dir");
    std::fs::create_dir_all(&cwd).expect("cwd");
    let settings = home.join(".claude/settings.json");
    let global = home.join(".claude.json");
    std::fs::write(&settings, "{\"kept\":true}\n").expect("settings");
    std::fs::write(&global, "{\"kept\":true}\n").expect("global");
    let before_settings = sha(&settings);
    let before_global = sha(&global);
    let mux = Arc::new(FakeMux::new());
    let ready = support::read(&support::fixtures().join("2.1.286/claude-020-ready.txt"));
    mux.set_capture(support::capture(&ready)).await;
    let adapter = ClaudeAdapter::new(support::shared(&mux), root.join("seats"), home);
    let binding = crewmux_adapters::binding_for(SeatId::new(), cwd.clone(), "seat__rig");
    adapter.launch(&binding, opts()).await.expect("launch");
    assert_eq!(sha(&settings), before_settings);
    assert_eq!(sha(&global), before_global);
    assert!(!cwd.join(".claude/settings.json").exists());
    assert!(!cwd.join(".claude/settings.local.json").exists());
    drop(std::fs::remove_dir_all(&root));
}

fn sha(path: &std::path::Path) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(std::fs::read(path).expect("read")).into()
}

fn opts() -> LaunchOpts {
    LaunchOpts {
        session_name: "seat__rig".to_owned(),
        env: BTreeMap::new(),
        resume: None,
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
